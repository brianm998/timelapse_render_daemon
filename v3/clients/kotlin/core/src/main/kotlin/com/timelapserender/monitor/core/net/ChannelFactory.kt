// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.net

import io.grpc.CallOptions
import io.grpc.Channel
import io.grpc.ClientCall
import io.grpc.ClientInterceptor
import io.grpc.ForwardingClientCall
import io.grpc.ManagedChannel
import io.grpc.Metadata
import io.grpc.MethodDescriptor
import io.grpc.okhttp.OkHttpChannelBuilder
import java.util.concurrent.TimeUnit

object ChannelFactory {
    const val KEEP_ALIVE_SECONDS = 30L

    fun create(endpoint: ServerEndpoint): ManagedChannel {
        val builder = OkHttpChannelBuilder.forAddress(endpoint.host, endpoint.port)
        when (val tls = endpoint.tls) {
            TlsMode.Plaintext -> builder.usePlaintext()
            is TlsMode.Pinned -> builder
                .useTransportSecurity()
                .sslSocketFactory(pinnedSslContext(tls.fingerprintSha256Hex).socketFactory)
                .hostnameVerifier(AnyNameHostnameVerifier)
        }
        return builder.keepAliveTime(KEEP_ALIVE_SECONDS, TimeUnit.SECONDS).build()
    }
}

/** Adds `authorization: Bearer <token>` to every call, only when [token] is set. */
class TokenInterceptor(private val token: String?) : ClientInterceptor {
    override fun <ReqT, RespT> interceptCall(
        method: MethodDescriptor<ReqT, RespT>,
        callOptions: CallOptions,
        next: Channel,
    ): ClientCall<ReqT, RespT> {
        val delegate = next.newCall(method, callOptions)
        if (token.isNullOrEmpty()) return delegate
        return object : ForwardingClientCall.SimpleForwardingClientCall<ReqT, RespT>(delegate) {
            override fun start(responseListener: Listener<RespT>, headers: Metadata) {
                headers.put(AUTHORIZATION, "Bearer $token")
                super.start(responseListener, headers)
            }
        }
    }

    companion object {
        val AUTHORIZATION: Metadata.Key<String> =
            Metadata.Key.of("authorization", Metadata.ASCII_STRING_MARSHALLER)
    }
}
