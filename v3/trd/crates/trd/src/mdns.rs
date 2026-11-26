// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! `_trd._tcp` advertisement (§8.4): the pure builder of the service, the registrar that talks to
//! `mdns-sd`, and the loop that registers again when the LAN port or the IPv4 addresses change.
//!
//! Real multicast happens only in [`MdnsSdRegistrar`], which refuses to exist under `cfg(test)`:
//! unit tests drive [`Advertiser`] with a fake registrar and a fake address source.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceInfo, UnregisterStatus};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use trd_core::problems::{Problem, Severity};

use crate::server::Shared;
use crate::tls;

/// The DNS-SD service type the daemon advertises (§8.4).
pub const SERVICE_TYPE: &str = "_trd._tcp.local.";
/// The `api` TXT record: the protocol's API version (§7).
pub const API_VERSION: &str = "1";
/// The id of the problem of a registration that failed (`PROBLEM_MDNS_FAILED`, §8.4).
pub const ID_MDNS: &str = "mdns";
/// How often the advertisement checks the LAN port and the IPv4 addresses (§8.4).
const CHECK_EVERY: Duration = Duration::from_secs(30);
/// How long a withdrawal waits for `mdns-sd` to send its goodbye packets.
const UNREGISTER_WAIT: Duration = Duration::from_secs(5);

/// Whether the daemon advertises at all: `--no-mdns` turns it off, and so does a LAN listener that
/// is off (§8.4).
pub fn wanted(mdns: bool, lan: Option<SocketAddr>) -> bool {
    mdns && lan.is_some()
}

/// The instance name for `advertised` (§8.4): every dot and whitespace character becomes `-`.
pub fn instance_name(advertised: &str) -> String {
    advertised
        .chars()
        .map(|c| {
            if c == '.' || c.is_whitespace() {
                '-'
            } else {
                c
            }
        })
        .collect()
}

/// What the advertisement says about this server. It does not change while the daemon runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advertised {
    /// `ServerInfo.server_id`, the TXT record `id`.
    pub server_id: String,
    /// `ServerInfo.name`: the instance name is derived from it, and it is the TXT record `name`.
    pub server_name: String,
    /// The TLS certificate's fingerprint, the TXT record `fp`.
    pub fingerprint: String,
    /// The trd version, the TXT record `ver`.
    pub version: String,
}

/// The service as `mdns-sd` sees it: instance `<instance>._trd._tcp.local.`, host
/// `<instance>.local.`, the LAN `port`, the `addresses`, and the TXT records `id`, `name`, `fp`,
/// `port`, `ver` and `api` (§8.4). `addresses` must not be empty.
pub fn service_info(
    advertised: &Advertised,
    port: u16,
    addresses: &[Ipv4Addr],
) -> Result<ServiceInfo, String> {
    let instance = instance_name(&advertised.server_name);
    let host = format!("{instance}.local.");
    let ips: Vec<IpAddr> = addresses.iter().copied().map(IpAddr::V4).collect();
    let txt: HashMap<String, String> = HashMap::from([
        ("id".to_owned(), advertised.server_id.clone()),
        ("name".to_owned(), advertised.server_name.clone()),
        ("fp".to_owned(), advertised.fingerprint.clone()),
        ("port".to_owned(), port.to_string()),
        ("ver".to_owned(), advertised.version.clone()),
        ("api".to_owned(), API_VERSION.to_owned()),
    ]);
    ServiceInfo::new(SERVICE_TYPE, &instance, &host, ips.as_slice(), port, txt)
        .map_err(|err| err.to_string())
}

/// The problem of a registration that failed: `PROBLEM_MDNS_FAILED`, with `error` as its detail.
pub fn mdns_problem(error: &str) -> Problem {
    Problem {
        id: ID_MDNS.to_owned(),
        severity: Severity::Error,
        kind: "PROBLEM_MDNS_FAILED",
        title: "The service is not advertised on the network".to_owned(),
        detail: format!(
            "trd cannot advertise itself over mDNS: {error}. Devices that look for trd on the local network cannot find it until this works. trd tries again every 30 seconds."
        ),
        fix: "Check that the network allows multicast DNS (UDP port 5353) and that trd may use the local network, or start trd with --no-mdns.".to_owned(),
        paths: Vec::new(),
        since_ms: 0,
    }
}

/// Sets the mDNS problem: raised with `error` as its detail, or removed when `error` is `None`.
fn set_problem(problems: &Mutex<Vec<Problem>>, error: Option<&str>) {
    let mut list = problems.lock().unwrap_or_else(PoisonError::into_inner);
    list.retain(|problem| problem.id != ID_MDNS);
    if let Some(error) = error {
        list.push(mdns_problem(error));
    }
}

/// Publishes and withdraws services. Registering a name again replaces its earlier registration.
pub trait Registrar {
    /// Publishes `info`.
    fn register(&mut self, info: &ServiceInfo) -> Result<(), String>;
    /// Withdraws the service `fullname`, sending its goodbye packets.
    fn unregister(&mut self, fullname: &str) -> Result<(), String>;
}

/// Where the IPv4 addresses of this machine come from, read at each check.
pub trait AddressSource {
    fn ipv4(&self) -> Vec<Ipv4Addr>;
}

/// The machine's real non-loopback IPv4 addresses.
pub struct LanAddresses;

impl AddressSource for LanAddresses {
    fn ipv4(&self) -> Vec<Ipv4Addr> {
        tls::lan_ipv4_addresses()
            .iter()
            .filter_map(|address| address.parse().ok())
            .collect()
    }
}

/// The registrar that talks to `mdns-sd`. It is the only code that creates a `ServiceDaemon`.
pub struct MdnsSdRegistrar {
    daemon: ServiceDaemon,
}

impl MdnsSdRegistrar {
    /// Starts the mDNS daemon. Real multicast: it is refused in unit tests, which use a fake.
    // The assertion is on `cfg!(test)`, which is a constant by design: it must panic at run time in
    // a unit test, so it cannot become a compile-time const assertion.
    #[allow(clippy::assertions_on_constants)]
    pub fn new() -> Result<Self, String> {
        assert!(
            !cfg!(test),
            "MdnsSdRegistrar uses real multicast; tests use FakeRegistrar"
        );
        ServiceDaemon::new()
            .map(|daemon| MdnsSdRegistrar { daemon })
            .map_err(|err| err.to_string())
    }
}

impl Registrar for MdnsSdRegistrar {
    fn register(&mut self, info: &ServiceInfo) -> Result<(), String> {
        self.daemon
            .register(info.clone())
            .map_err(|err| err.to_string())
    }

    fn unregister(&mut self, fullname: &str) -> Result<(), String> {
        let status = self
            .daemon
            .unregister(fullname)
            .map_err(|err| err.to_string())?;
        // Waits until the goodbye packets are sent, so that the service is gone when trd exits.
        match status.recv_timeout(UNREGISTER_WAIT) {
            Ok(UnregisterStatus::OK | UnregisterStatus::NotFound) => Ok(()),
            Err(err) => Err(format!("no answer to the withdrawal: {err}")),
        }
    }
}

impl Drop for MdnsSdRegistrar {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}

/// What the advertiser has registered: the name, the port and the address set.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Published {
    fullname: String,
    port: u16,
    addresses: Vec<Ipv4Addr>,
}

/// Keeps the service registered with the port and addresses the machine has now (§8.4).
pub struct Advertiser<R, A> {
    advertised: Advertised,
    registrar: R,
    addresses: A,
    published: Option<Published>,
    problems: Arc<Mutex<Vec<Problem>>>,
}

impl<R: Registrar, A: AddressSource> Advertiser<R, A> {
    pub fn new(
        advertised: Advertised,
        registrar: R,
        addresses: A,
        problems: Arc<Mutex<Vec<Problem>>>,
    ) -> Self {
        Advertiser {
            advertised,
            registrar,
            addresses,
            published: None,
            problems,
        }
    }

    /// One check. Registers when nothing is registered yet, or when the port or the address set
    /// differs from what is registered. Withdraws when there is nothing to advertise: no port yet,
    /// or no address. A failure raises the mDNS problem; a successful registration clears it.
    pub fn sync(&mut self, port: u16) {
        let mut addresses = self.addresses.ipv4();
        addresses.sort();
        addresses.dedup();
        if port == 0 || addresses.is_empty() {
            self.withdraw();
            return;
        }
        if self
            .published
            .as_ref()
            .is_some_and(|published| published.port == port && published.addresses == addresses)
        {
            return;
        }
        let info = match service_info(&self.advertised, port, &addresses) {
            Ok(info) => info,
            Err(err) => return set_problem(&self.problems, Some(&err)),
        };
        match self.registrar.register(&info) {
            Ok(()) => {
                self.published = Some(Published {
                    fullname: info.get_fullname().to_owned(),
                    port,
                    addresses,
                });
                set_problem(&self.problems, None);
            }
            Err(err) => set_problem(&self.problems, Some(&err)),
        }
    }

    /// Withdraws the service when it is registered. Called at shutdown.
    pub fn stop(&mut self) {
        self.withdraw();
    }

    fn withdraw(&mut self) {
        if let Some(published) = self.published.take()
            && let Err(err) = self.registrar.unregister(&published.fullname)
        {
            set_problem(&self.problems, Some(&err));
        }
    }
}

/// Runs the advertisement: checks at once and then every [`CHECK_EVERY`], with the LAN port from
/// `lan_port`, until `stop` is set. Then withdraws the service.
pub async fn run<R, A>(
    mut advertiser: Advertiser<R, A>,
    lan_port: impl Fn() -> u16,
    mut stop: watch::Receiver<bool>,
) where
    R: Registrar,
    A: AddressSource,
{
    let mut check = tokio::time::interval(CHECK_EVERY);
    loop {
        tokio::select! {
            _ = check.tick() => advertiser.sync(lan_port()),
            _ = stop.wait_for(|stopped| *stopped) => break,
        }
    }
    advertiser.stop();
}

/// Starts the advertisement of the daemon `shared` describes (§8.4). The task ends, after it has
/// withdrawn the service, when `stop` is set.
pub fn spawn(shared: Arc<Shared>, stop: watch::Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let advertised = Advertised {
            server_id: shared.server_id.clone(),
            server_name: shared.server_name.clone(),
            fingerprint: shared
                .identity
                .as_ref()
                .map(|identity| identity.fingerprint.clone())
                .unwrap_or_default(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        let problems = shared.external_problems.clone();
        match MdnsSdRegistrar::new() {
            Ok(registrar) => {
                let advertiser = Advertiser::new(advertised, registrar, LanAddresses, problems);
                let port_source = shared.clone();
                let lan_port = move || port_source.lan_port.load(AtomicOrdering::SeqCst);
                run(advertiser, lan_port, stop).await;
            }
            Err(err) => set_problem(&problems, Some(&err)),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One call the fake registrar received.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Register {
            fullname: String,
            port: u16,
            addresses: Vec<Ipv4Addr>,
        },
        Unregister(String),
    }

    /// Records the calls. `fail` makes every registration fail with its message while it is set.
    #[derive(Clone, Default)]
    struct FakeRegistrar {
        calls: Arc<Mutex<Vec<Call>>>,
        fail: Arc<Mutex<Option<String>>>,
    }

    impl Registrar for FakeRegistrar {
        fn register(&mut self, info: &ServiceInfo) -> Result<(), String> {
            if let Some(err) = self.fail.lock().expect("the fail flag").clone() {
                return Err(err);
            }
            let mut addresses: Vec<Ipv4Addr> =
                info.get_addresses_v4().into_iter().copied().collect();
            addresses.sort();
            self.calls.lock().expect("the calls").push(Call::Register {
                fullname: info.get_fullname().to_owned(),
                port: info.get_port(),
                addresses,
            });
            Ok(())
        }

        fn unregister(&mut self, fullname: &str) -> Result<(), String> {
            self.calls
                .lock()
                .expect("the calls")
                .push(Call::Unregister(fullname.to_owned()));
            Ok(())
        }
    }

    /// The address set the test sets.
    #[derive(Clone, Default)]
    struct FakeAddresses(Arc<Mutex<Vec<Ipv4Addr>>>);

    impl FakeAddresses {
        fn set(&self, addresses: &[Ipv4Addr]) {
            *self.0.lock().expect("the addresses") = addresses.to_vec();
        }
    }

    impl AddressSource for FakeAddresses {
        fn ipv4(&self) -> Vec<Ipv4Addr> {
            self.0.lock().expect("the addresses").clone()
        }
    }

    fn advertised() -> Advertised {
        Advertised {
            server_id: "5f1c2e7a-0000-4000-8000-000000000001".to_owned(),
            server_name: "Studio One.local".to_owned(),
            fingerprint: "ab12cd34".to_owned(),
            version: "3.0.0".to_owned(),
        }
    }

    fn lan(a: u8) -> Ipv4Addr {
        Ipv4Addr::new(192, 168, 1, a)
    }

    fn problems() -> Arc<Mutex<Vec<Problem>>> {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn txt(info: &ServiceInfo, key: &str) -> Option<String> {
        info.get_property_val_str(key).map(str::to_owned)
    }

    /// The instance name replaces every dot and whitespace character with `-`, and no other.
    #[test]
    fn instance_name_replaces_dots_and_whitespace() {
        assert_eq!(instance_name("Studio One.local"), "Studio-One-local");
        assert_eq!(instance_name("a\tb\nc.d"), "a-b-c-d");
        assert_eq!(instance_name("trd_host-2"), "trd_host-2");
    }

    /// A1: the builder's name, host, port, addresses and exact TXT records.
    #[test]
    fn builder_gives_the_instance_host_port_addresses_and_txt_records() {
        let info = service_info(&advertised(), 50072, &[lan(10), lan(11)]).expect("a service");
        assert_eq!(info.get_fullname(), "Studio-One-local._trd._tcp.local.");
        assert_eq!(info.get_hostname(), "Studio-One-local.local.");
        assert_eq!(info.get_port(), 50072);
        let mut addresses: Vec<Ipv4Addr> = info.get_addresses_v4().into_iter().copied().collect();
        addresses.sort();
        assert_eq!(addresses, vec![lan(10), lan(11)]);
        assert_eq!(
            txt(&info, "id").as_deref(),
            Some("5f1c2e7a-0000-4000-8000-000000000001")
        );
        assert_eq!(txt(&info, "name").as_deref(), Some("Studio One.local"));
        assert_eq!(txt(&info, "fp").as_deref(), Some("ab12cd34"));
        assert_eq!(txt(&info, "port").as_deref(), Some("50072"));
        assert_eq!(txt(&info, "ver").as_deref(), Some("3.0.0"));
        assert_eq!(txt(&info, "api").as_deref(), Some("1"));
        assert!(
            info.get_property("missing").is_none(),
            "no other TXT record"
        );
    }

    /// A2: an unchanged address set and port do not register again.
    #[test]
    fn an_unchanged_address_set_is_not_registered_again() {
        let registrar = FakeRegistrar::default();
        let addresses = FakeAddresses::default();
        addresses.set(&[lan(10)]);
        let mut advertiser = Advertiser::new(
            advertised(),
            registrar.clone(),
            addresses.clone(),
            problems(),
        );
        advertiser.sync(50072);
        advertiser.sync(50072);
        addresses.set(&[lan(10), lan(10)]);
        advertiser.sync(50072);
        let calls = registrar.calls.lock().expect("the calls").clone();
        assert_eq!(
            calls,
            vec![Call::Register {
                fullname: "Studio-One-local._trd._tcp.local.".to_owned(),
                port: 50072,
                addresses: vec![lan(10)],
            }]
        );
    }

    /// A2: a changed address set registers exactly once more, with the new addresses. A changed
    /// port does the same.
    #[test]
    fn a_changed_address_set_is_registered_once_with_the_new_addresses() {
        let registrar = FakeRegistrar::default();
        let addresses = FakeAddresses::default();
        addresses.set(&[lan(10)]);
        let mut advertiser = Advertiser::new(
            advertised(),
            registrar.clone(),
            addresses.clone(),
            problems(),
        );
        advertiser.sync(50072);
        addresses.set(&[lan(10), lan(20)]);
        advertiser.sync(50072);
        advertiser.sync(50072);
        assert_eq!(registrar.calls.lock().expect("the calls").len(), 2);
        let last = registrar.calls.lock().expect("the calls").last().cloned();
        assert_eq!(
            last,
            Some(Call::Register {
                fullname: "Studio-One-local._trd._tcp.local.".to_owned(),
                port: 50072,
                addresses: vec![lan(10), lan(20)],
            })
        );
        advertiser.sync(50073);
        assert_eq!(
            registrar.calls.lock().expect("the calls").len(),
            3,
            "a new port registers"
        );
    }

    /// Nothing is advertised without a port or an address; an address set that empties withdraws
    /// the registration, and stop withdraws it too.
    #[test]
    fn nothing_is_advertised_without_a_port_or_an_address() {
        let registrar = FakeRegistrar::default();
        let addresses = FakeAddresses::default();
        let mut advertiser = Advertiser::new(
            advertised(),
            registrar.clone(),
            addresses.clone(),
            problems(),
        );
        advertiser.sync(50072);
        addresses.set(&[lan(10)]);
        advertiser.sync(0);
        assert!(registrar.calls.lock().expect("the calls").is_empty());
        advertiser.sync(50072);
        addresses.set(&[]);
        advertiser.sync(50072);
        advertiser.stop();
        let calls = registrar.calls.lock().expect("the calls").clone();
        assert_eq!(
            calls.len(),
            2,
            "one registration, then one withdrawal: {calls:?}"
        );
        assert!(matches!(calls[0], Call::Register { .. }));
        assert_eq!(
            calls[1],
            Call::Unregister("Studio-One-local._trd._tcp.local.".to_owned())
        );
    }

    /// A failed registration raises PROBLEM_MDNS_FAILED, and the next successful one clears it.
    #[test]
    fn a_failed_registration_raises_the_problem_until_one_succeeds() {
        let registrar = FakeRegistrar::default();
        let addresses = FakeAddresses::default();
        addresses.set(&[lan(10)]);
        let problems = problems();
        let mut advertiser =
            Advertiser::new(advertised(), registrar.clone(), addresses, problems.clone());
        *registrar.fail.lock().expect("the fail flag") = Some("network unreachable".to_owned());
        advertiser.sync(50072);
        let raised = problems.lock().expect("the problems").clone();
        assert_eq!(raised.len(), 1);
        assert_eq!(raised[0].id, "mdns");
        assert_eq!(raised[0].kind, "PROBLEM_MDNS_FAILED");
        assert!(raised[0].detail.contains("network unreachable"));

        *registrar.fail.lock().expect("the fail flag") = None;
        advertiser.sync(50072);
        assert!(problems.lock().expect("the problems").is_empty());
        assert_eq!(registrar.calls.lock().expect("the calls").len(), 1);
    }

    /// The real registrar is refused in unit tests: it would send multicast.
    #[test]
    #[should_panic(expected = "MdnsSdRegistrar uses real multicast")]
    fn registrar_refuses_in_unit_tests() {
        let _ = MdnsSdRegistrar::new();
    }

    /// `--no-mdns` and `--lan-bind off` each turn the advertisement off.
    #[test]
    fn mdns_is_wanted_only_with_mdns_on_and_a_lan_listener() {
        let lan_addr = SocketAddr::from(([127, 0, 0, 1], 0));
        assert!(wanted(true, Some(lan_addr)));
        assert!(!wanted(false, Some(lan_addr)), "--no-mdns");
        assert!(!wanted(true, None), "--lan-bind off");
        assert!(!wanted(false, None));
    }
}
