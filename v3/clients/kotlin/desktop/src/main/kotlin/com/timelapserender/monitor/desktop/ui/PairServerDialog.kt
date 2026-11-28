// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.net.certificateFingerprintSha256Hex
import com.timelapserender.monitor.core.pairing.PairingClient
import com.timelapserender.monitor.core.pairing.PairingException
import com.timelapserender.monitor.core.servers.ServerRegistry
import com.timelapserender.monitor.desktop.AppDialog
import com.timelapserender.monitor.desktop.AppState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.IOException
import java.net.InetSocketAddress
import java.net.Socket
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.X509TrustManager

const val PAIR_CODE_TAG = "pair-code"

object PairServerTags {
    const val HOST = "pair-host"
    const val PORT = "pair-port"
    const val FINGERPRINT = "pair-fingerprint"
}

private const val CODE_LENGTH = 6

/** `trd`'s default LAN port (overview §3). */
const val DEFAULT_LAN_PORT = ServerRegistry.DEFAULT_LOCAL_PORT + 1

/** The digits of a fingerprint shown by the desktop pairing dialog; a typed one needs at least these. */
const val FINGERPRINT_PREFIX_LENGTH = 16
private const val FINGERPRINT_LENGTH = 64
private const val PROBE_TIMEOUT_MS = 20_000

const val BAD_FINGERPRINT =
    "Enter at least the $FINGERPRINT_PREFIX_LENGTH hex digits of the fingerprint that the trd machine shows."

const val FINGERPRINT_MISMATCH =
    "The server at that address has a different fingerprint. Check it against the one the trd machine shows."

/** The name under which this desktop is listed on the server it pairs with. */
fun desktopDeviceName(hostName: String): String = "$hostName (desktop)"

/**
 * A typed fingerprint without its grouping (spaces or colons), in lowercase; null unless it is hex
 * with at least the [FINGERPRINT_PREFIX_LENGTH] digits the pairing dialog shows.
 */
fun normalizeFingerprint(typed: String): String? {
    val hex = typed.filterNot { it.isWhitespace() || it == ':' }.lowercase()
    val valid = hex.length in FINGERPRINT_PREFIX_LENGTH..FINGERPRINT_LENGTH &&
        hex.all { it in '0'..'9' || it in 'a'..'f' }
    return hex.takeIf { valid }
}

/**
 * The whole fingerprint of the certificate at [host]:[port], which must start with [prefix]: a
 * hand-entered server is pinned to it. A TLS handshake and nothing else; any other certificate
 * ends it.
 *
 * @throws PairingException with a message for the user.
 */
suspend fun probeFingerprint(host: String, port: Int, prefix: String): String = withContext(Dispatchers.IO) {
    var found: String? = null
    var mismatch = false
    val trust = object : X509TrustManager {
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {
            throw CertificateException("This trust manager does not authenticate clients")
        }

        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
            val leaf = chain?.firstOrNull() ?: throw CertificateException("The server sent no certificate")
            val actual = certificateFingerprintSha256Hex(leaf)
            if (!actual.startsWith(prefix)) {
                mismatch = true
                throw CertificateException(FINGERPRINT_MISMATCH)
            }
            found = actual
        }

        override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
    }
    val context = SSLContext.getInstance("TLS").apply { init(null, arrayOf(trust), null) }
    try {
        Socket().use { plain ->
            plain.connect(InetSocketAddress(host, port), PROBE_TIMEOUT_MS)
            plain.soTimeout = PROBE_TIMEOUT_MS
            (context.socketFactory.createSocket(plain, host, port, true) as SSLSocket).use { tls ->
                tls.sslParameters = tls.sslParameters.apply { applicationProtocols = arrayOf("h2") }
                tls.startHandshake()
            }
        }
    } catch (e: IOException) {
        throw PairingException(if (mismatch) FINGERPRINT_MISMATCH else PairingClient.CONNECTION_FAILED, e)
    } catch (e: IllegalArgumentException) {
        // A port or host name that cannot be an address.
        throw PairingException(PairingClient.CONNECTION_FAILED, e)
    }
    found ?: throw PairingException(PairingClient.CONNECTION_FAILED)
}

/** Overview §6.6: type the 6-digit code shown on a discovered or already known server. */
@Composable
fun PairServerDialog(dialog: AppDialog.Pair, app: AppState) {
    var code by remember(dialog.server.serverId) { mutableStateOf("") }
    PairingFrame(
        title = "Pair with ${dialog.server.name}",
        busy = dialog.busy,
        error = dialog.error,
        complete = code.length == CODE_LENGTH,
        onPair = { app.submitPairing(code) },
        onDismiss = app::dismissDialog,
    ) {
        Text("Enter the 6-digit code shown on the trd machine.")
        CodeField(code, !dialog.busy) { code = it }
    }
}

/** A server the network does not announce: its address and fingerprint are typed in as well. */
@Composable
fun PairByAddressDialog(dialog: AppDialog.PairByAddress, app: AppState) {
    var host by remember { mutableStateOf("") }
    var port by remember { mutableStateOf(DEFAULT_LAN_PORT.toString()) }
    var fingerprint by remember { mutableStateOf("") }
    var code by remember { mutableStateOf("") }
    val portNumber = port.toIntOrNull()?.takeIf { it in 1..65535 }
    PairingFrame(
        title = "Add a server",
        busy = dialog.busy,
        error = dialog.error,
        complete = host.isNotBlank() && portNumber != null && normalizeFingerprint(fingerprint) != null &&
            code.length == CODE_LENGTH,
        onPair = { app.submitPairingByAddress(host, portNumber ?: 0, fingerprint, code) },
        onDismiss = app::dismissDialog,
    ) {
        Text("Pair a phone… on the trd machine shows these.")
        OutlinedTextField(
            value = host,
            onValueChange = { host = it },
            label = { Text("Address") },
            singleLine = true,
            enabled = !dialog.busy,
            modifier = Modifier.testTag(PairServerTags.HOST),
        )
        OutlinedTextField(
            value = port,
            onValueChange = { typed -> port = typed.filter { it.isDigit() }.take(5) },
            label = { Text("Port") },
            singleLine = true,
            enabled = !dialog.busy,
            modifier = Modifier.testTag(PairServerTags.PORT),
        )
        OutlinedTextField(
            value = fingerprint,
            onValueChange = { fingerprint = it },
            label = { Text("Fingerprint (first 16 hex digits)") },
            singleLine = true,
            enabled = !dialog.busy,
            modifier = Modifier.testTag(PairServerTags.FINGERPRINT),
        )
        Text("6-digit code")
        CodeField(code, !dialog.busy) { code = it }
    }
}

@Composable
private fun CodeField(code: String, enabled: Boolean, onChange: (String) -> Unit) {
    OutlinedTextField(
        value = code,
        onValueChange = { typed -> onChange(typed.filter { it.isDigit() }.take(CODE_LENGTH)) },
        singleLine = true,
        enabled = enabled,
        modifier = Modifier.testTag(PAIR_CODE_TAG),
    )
}

@Composable
private fun PairingFrame(
    title: String,
    busy: Boolean,
    error: String?,
    complete: Boolean,
    onPair: () -> Unit,
    onDismiss: () -> Unit,
    fields: @Composable () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column(
                Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                fields()
                error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = {
            TextButton(onClick = onPair, enabled = !busy && complete) { Text(if (busy) "Pairing…" else "Pair") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
