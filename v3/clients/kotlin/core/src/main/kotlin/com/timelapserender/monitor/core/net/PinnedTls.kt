// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.net

import java.security.MessageDigest
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import javax.net.ssl.HostnameVerifier
import javax.net.ssl.SSLContext
import javax.net.ssl.X509TrustManager

/** Lowercase hex SHA-256 over the DER encoding of [cert]. */
fun certificateFingerprintSha256Hex(cert: X509Certificate): String =
    MessageDigest.getInstance("SHA-256").digest(cert.encoded).joinToString("") { "%02x".format(it) }

/** Accepts exactly the leaf certificate whose SHA-256 fingerprint equals the pin; rejects everything else. */
class PinnedTrustManager(pinHex: String) : X509TrustManager {
    private val pin = pinHex.trim().lowercase()

    override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {
        throw CertificateException("This trust manager does not authenticate clients")
    }

    override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
        val leaf = chain?.firstOrNull() ?: throw CertificateException("The server sent no certificate")
        val actual = certificateFingerprintSha256Hex(leaf)
        if (!actual.equals(pin, ignoreCase = true)) {
            throw CertificateException("Server certificate does not match the pinned fingerprint")
        }
    }

    override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
}

fun pinnedSslContext(pinHex: String): SSLContext =
    SSLContext.getInstance("TLS").apply { init(null, arrayOf(PinnedTrustManager(pinHex)), null) }

/** The pin is the identity, so any host name is accepted. */
val AnyNameHostnameVerifier: HostnameVerifier = HostnameVerifier { _, _ -> true }
