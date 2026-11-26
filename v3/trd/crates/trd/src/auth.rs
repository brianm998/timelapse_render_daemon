// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! Per-device tokens for the LAN listener (§8.2).
//!
//! The LAN listener's interceptor turns the `authorization: Bearer <token>` metadata into a
//! [`Caller`] extension when the token's SHA-256 matches a non-revoked `devices` row. Each RPC except
//! `GetServerInfo` and `Pair` then calls [`authorize`], which refuses a LAN call without a caller.
//! A caller carries a receiver that turns `true` when its device is revoked; the streams it opens
//! end with `UNAUTHENTICATED` when it does (§8.3).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};

use sha2::{Digest, Sha256};
use tokio::sync::watch;
use tokio_stream::Stream;
use tonic::metadata::MetadataMap;
use tonic::{Request, Status};
use trd_core::store::{Device, Store};

use crate::server::{Listener, Shared, now_ms};

/// `last_seen` is written at most this often (§8.2).
pub const TOUCH_INTERVAL_MS: i64 = 60_000;

/// The device a LAN call came from, set by the LAN interceptor.
#[derive(Debug, Clone)]
pub struct Caller {
    /// `true` once the device is revoked.
    pub revoked: watch::Receiver<bool>,
}

/// Lowercase hex SHA-256 of a device token. Only this hash is stored (§8.3).
pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// The token of an `authorization: Bearer <token>` header, when there is a non-empty one.
pub fn bearer_token(metadata: &MetadataMap) -> Option<&str> {
    metadata
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
}

/// Whether `last_seen` is written now: never seen, or the last write is a minute old (§8.2).
pub fn touch_due(last_seen: Option<i64>, now: i64) -> bool {
    match last_seen {
        None => true,
        Some(seen) => now.saturating_sub(seen) >= TOUCH_INTERVAL_MS,
    }
}

/// The caller behind `token`: the device whose token hashes to the same value, when it is not
/// revoked. Writes `last_seen` when [`touch_due`]. `None` when no caller matches.
pub fn authenticate(
    store: &Store,
    revocations: &Revocations,
    token: Option<&str>,
    now: i64,
) -> Result<Option<Caller>, Status> {
    let Some(token) = token else {
        return Ok(None);
    };
    let device = store
        .device_by_token_hash(&token_hash(token))
        .map_err(|err| Status::internal(err.to_string()))?;
    let Some(device) = device.filter(|device| !device.revoked) else {
        return Ok(None);
    };
    if touch_due(device.last_seen, now) {
        store
            .touch_device(&device.id, now)
            .map_err(|err| Status::internal(err.to_string()))?;
    }
    Ok(Some(caller_of(&device, revocations)))
}

fn caller_of(device: &Device, revocations: &Revocations) -> Caller {
    Caller {
        revoked: revocations.subscribe(&device.id),
    }
}

/// The interceptor of the LAN listener: marks the request as LAN and attaches its [`Caller`], when
/// its bearer token names a paired device. It never refuses a request; [`authorize`] does.
pub fn lan_interceptor(
    shared: Arc<Shared>,
) -> impl FnMut(Request<()>) -> Result<Request<()>, Status> + Clone + Send + Sync + 'static {
    move |mut request: Request<()>| {
        request.extensions_mut().insert(Listener::Lan);
        let token = bearer_token(request.metadata()).map(str::to_owned);
        let caller = authenticate(
            &shared.store,
            &shared.revocations,
            token.as_deref(),
            now_ms(),
        )?;
        if let Some(caller) = caller {
            request.extensions_mut().insert(caller);
        }
        Ok(request)
    }
}

/// `UNAUTHENTICATED` for a LAN call that has no paired device's caller. Loopback calls pass.
pub fn authorize<T>(request: &Request<T>) -> Result<(), Status> {
    let on_lan = request.extensions().get::<Listener>().copied() != Some(Listener::Loopback);
    if !on_lan || request.extensions().get::<Caller>().is_some() {
        return Ok(());
    }
    Err(Status::unauthenticated(
        "a LAN call needs the bearer token of a paired device",
    ))
}

/// The revocation signal of the caller behind `request`, if it is a paired device's LAN call.
pub fn revocation_of<T>(request: &Request<T>) -> Option<watch::Receiver<bool>> {
    request
        .extensions()
        .get::<Caller>()
        .map(|caller| caller.revoked.clone())
}

/// One revocation signal per device. A device's streams hold a receiver; revoking sends `true` to
/// all of them. A device with no stream still gets a signal, so a revoke that lands before its
/// next subscription is seen by it.
#[derive(Debug, Default)]
pub struct Revocations {
    senders: Mutex<HashMap<String, watch::Sender<bool>>>,
}

impl Revocations {
    /// A receiver for `device_id`'s revocation signal.
    pub fn subscribe(&self, device_id: &str) -> watch::Receiver<bool> {
        let mut senders = self.senders.lock().unwrap_or_else(PoisonError::into_inner);
        senders
            .entry(device_id.to_owned())
            .or_insert_with(|| watch::channel(false).0)
            .subscribe()
    }

    /// Signals that `device_id` is revoked: its open streams end.
    pub fn revoke(&self, device_id: &str) {
        let mut senders = self.senders.lock().unwrap_or_else(PoisonError::into_inner);
        senders
            .entry(device_id.to_owned())
            .or_insert_with(|| watch::channel(false).0)
            .send_replace(true);
    }
}

/// A stream that ends with `UNAUTHENTICATED` once its device is revoked. With no revocation
/// signal (a loopback stream) it is the inner stream.
pub struct UntilRevoked<S> {
    inner: S,
    revoked: Pin<Box<dyn Future<Output = ()> + Send>>,
    ended: bool,
}

/// Wraps `stream` so that it ends when `revoked` turns `true`.
pub fn until_revoked<S>(stream: S, revoked: Option<watch::Receiver<bool>>) -> UntilRevoked<S> {
    let revoked: Pin<Box<dyn Future<Output = ()> + Send>> = match revoked {
        Some(mut signal) => Box::pin(async move {
            let _ = signal.wait_for(|revoked| *revoked).await;
        }),
        None => Box::pin(std::future::pending()),
    };
    UntilRevoked {
        inner: stream,
        revoked,
        ended: false,
    }
}

impl<T, S> Stream for UntilRevoked<S>
where
    S: Stream<Item = Result<T, Status>> + Unpin,
{
    type Item = Result<T, Status>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.ended {
            return Poll::Ready(None);
        }
        if this.revoked.as_mut().poll(cx).is_ready() {
            this.ended = true;
            return Poll::Ready(Some(Err(Status::unauthenticated("the device was revoked"))));
        }
        let next = Pin::new(&mut this.inner).poll_next(cx);
        if matches!(next, Poll::Ready(None)) {
            this.ended = true;
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio_stream::StreamExt;
    use tonic::metadata::MetadataValue;

    use super::*;

    /// Stops a test that waits for a state that never comes. It is not a timing assertion.
    const HANG_GUARD: Duration = Duration::from_secs(60);

    fn store_in(dir: &std::path::Path) -> Store {
        Store::open(&dir.join("trd.sqlite3")).expect("a store in a temporary folder")
    }

    fn device(id: &str, token: &str) -> Device {
        Device {
            id: id.to_owned(),
            name: "Phone".to_owned(),
            platform: "ios".to_owned(),
            token_sha256: token_hash(token),
            paired_at: 0,
            last_seen: None,
            revoked: false,
        }
    }

    #[test]
    fn touch_is_due_once_a_minute() {
        assert!(touch_due(None, 5));
        assert!(!touch_due(Some(1_000), 1_000 + TOUCH_INTERVAL_MS - 1));
        assert!(touch_due(Some(1_000), 1_000 + TOUCH_INTERVAL_MS));
        // A clock that went back does not write on every call.
        assert!(!touch_due(Some(1_000), 0));
    }

    #[test]
    fn tokens_are_stored_only_as_their_sha256() {
        let token = "secret-token";
        let hash = token_hash(token);
        assert_eq!(hash.len(), 64);
        assert!(!hash.contains(token));
        assert_eq!(hash, hex::encode(Sha256::digest(token.as_bytes())));
    }

    #[test]
    fn the_bearer_token_is_read_from_the_authorization_header() {
        let mut metadata = MetadataMap::new();
        assert_eq!(bearer_token(&metadata), None);
        metadata.insert("authorization", MetadataValue::from_static("Bearer abc"));
        assert_eq!(bearer_token(&metadata), Some("abc"));
        metadata.insert("authorization", MetadataValue::from_static("Bearer "));
        assert_eq!(bearer_token(&metadata), None, "an empty token is no token");
        metadata.insert("authorization", MetadataValue::from_static("Basic abc"));
        assert_eq!(bearer_token(&metadata), None);
    }

    #[test]
    fn authenticate_finds_only_a_paired_unrevoked_device() {
        let dir = tempfile::Builder::new()
            .prefix("trd-auth-unit-")
            .tempdir()
            .expect("a temporary directory");
        let store = store_in(dir.path());
        let revocations = Revocations::default();
        store
            .put_device(&device("dev-1", "token-1"))
            .expect("the device");

        assert!(
            authenticate(&store, &revocations, None, 0)
                .expect("no token")
                .is_none()
        );
        assert!(
            authenticate(&store, &revocations, Some("wrong"), 0)
                .expect("a wrong token")
                .is_none()
        );
        let caller = authenticate(&store, &revocations, Some("token-1"), 0)
            .expect("the token")
            .expect("the device");
        assert!(!*caller.revoked.borrow(), "the device is not revoked yet");

        let mut revoked = device("dev-1", "token-1");
        revoked.revoked = true;
        store.put_device(&revoked).expect("the revoked device");
        assert!(
            authenticate(&store, &revocations, Some("token-1"), 0)
                .expect("a revoked token")
                .is_none()
        );
    }

    /// `last_seen` is written on the first call and then at most once a minute.
    #[test]
    fn last_seen_is_written_at_most_once_a_minute() {
        let dir = tempfile::Builder::new()
            .prefix("trd-auth-unit-")
            .tempdir()
            .expect("a temporary directory");
        let store = store_in(dir.path());
        let revocations = Revocations::default();
        store
            .put_device(&device("dev-1", "token-1"))
            .expect("the device");
        let seen = |store: &Store| {
            store
                .device("dev-1")
                .expect("a read")
                .expect("the row")
                .last_seen
        };

        authenticate(&store, &revocations, Some("token-1"), 1_000).expect("a call");
        assert_eq!(seen(&store), Some(1_000));
        authenticate(
            &store,
            &revocations,
            Some("token-1"),
            1_000 + TOUCH_INTERVAL_MS - 1,
        )
        .expect("a call");
        assert_eq!(seen(&store), Some(1_000), "not yet a minute");
        authenticate(
            &store,
            &revocations,
            Some("token-1"),
            1_000 + TOUCH_INTERVAL_MS,
        )
        .expect("a call");
        assert_eq!(seen(&store), Some(1_000 + TOUCH_INTERVAL_MS));
    }

    #[test]
    fn authorize_refuses_a_lan_call_without_a_caller() {
        let mut request = Request::new(());
        request.extensions_mut().insert(Listener::Lan);
        assert_eq!(
            authorize(&request).expect_err("no caller").code(),
            tonic::Code::Unauthenticated
        );
        let mut loopback = Request::new(());
        loopback.extensions_mut().insert(Listener::Loopback);
        assert!(authorize(&loopback).is_ok());
    }

    #[tokio::test]
    async fn a_revoked_device_ends_its_stream_with_unauthenticated() {
        let revocations = Revocations::default();
        let signal = revocations.subscribe("dev-1");
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<u32, Status>>(4);
        tx.send(Ok(1)).await.expect("a message");
        let mut stream = until_revoked(
            tokio_stream::wrappers::ReceiverStream::new(rx),
            Some(signal),
        );
        assert_eq!(stream.next().await.expect("an item").expect("a message"), 1);
        revocations.revoke("dev-1");
        let ended = tokio::time::timeout(HANG_GUARD, stream.next())
            .await
            .expect("the revoke ends the stream")
            .expect("an error item");
        assert_eq!(
            ended.expect_err("the revoke").code(),
            tonic::Code::Unauthenticated
        );
        assert!(stream.next().await.is_none(), "the stream is over");
    }

    #[tokio::test]
    async fn a_stream_without_a_signal_ends_with_its_inner_stream() {
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<u32, Status>>(4);
        drop(tx);
        let mut stream = until_revoked(tokio_stream::wrappers::ReceiverStream::new(rx), None);
        assert!(stream.next().await.is_none());
    }

    #[test]
    fn revoking_before_a_subscription_is_still_seen() {
        let revocations = Revocations::default();
        revocations.revoke("dev-2");
        assert!(*revocations.subscribe("dev-2").borrow());
        assert!(!*revocations.subscribe("dev-3").borrow());
    }
}
