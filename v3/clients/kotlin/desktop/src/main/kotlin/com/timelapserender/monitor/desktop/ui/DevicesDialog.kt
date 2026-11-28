// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.timelapserender.monitor.core.state.DeviceRowUi
import com.timelapserender.monitor.core.state.DevicesModel
import com.timelapserender.monitor.core.state.PairingUi

object DevicesTags {
    const val CODE = "devices-code"
    const val COUNTDOWN = "devices-countdown"
    const val FINGERPRINT = "devices-fingerprint"
    const val NEW_CODE = "devices-new-code"
    const val CONFIRM_REVOKE = "devices-confirm-revoke"

    fun revoke(deviceId: String) = "devices-revoke-$deviceId"
}

const val CODE_EXPIRED = "This code has expired. Ask for a new one."

/** While a code is shown the device list is read again this often, so a new phone appears. */
private const val REFRESH_SECONDS = 3L

/** `123456` as `123 456`. */
fun groupedCode(code: String): String = code.chunked(3).joinToString(" ")

/** Hex digits in fours. */
fun groupedFingerprint(hex: String): String = hex.chunked(4).joinToString(" ")

/**
 * "Pair a phone…" (overview §6.7): a pairing code with its countdown and what a phone needs to add
 * the server by hand, then the paired devices. Opening it asks for a code.
 */
@Composable
fun DevicesDialog(model: DevicesModel, onDismiss: () -> Unit) {
    val ui by model.state.collectAsState()
    var revoking by remember(model) { mutableStateOf<DeviceRowUi?>(null) }
    LaunchedEffect(model) {
        model.startPairing()
        model.refresh()
    }
    val remaining = ui.pairing?.remainingSeconds
    LaunchedEffect(model, remaining) {
        if (remaining != null && remaining % REFRESH_SECONDS == 0L) model.refresh()
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Pair a phone") },
        text = {
            Column(
                Modifier.width(420.dp).heightIn(max = 520.dp).verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                ui.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                val pairing = ui.pairing
                if (pairing != null) {
                    PairingCode(pairing)
                } else if (ui.error == null) {
                    Text("Asking for a code…")
                }
                OutlinedButton(onClick = model::startPairing, modifier = Modifier.testTag(DevicesTags.NEW_CODE)) {
                    Text("New code")
                }
                HorizontalDivider()
                Text("Paired devices", style = MaterialTheme.typography.titleMedium)
                if (ui.devices.isEmpty()) Text("No paired devices.")
                for (device in ui.devices) {
                    DeviceRow(device) { revoking = device }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Close") } },
    )
    revoking?.let { device ->
        AlertDialog(
            onDismissRequest = { revoking = null },
            title = { Text("Revoke ${device.name}?") },
            text = { Text("It is disconnected and has to be paired again.") },
            confirmButton = {
                TextButton(
                    onClick = {
                        revoking = null
                        model.revoke(device.deviceId)
                    },
                    modifier = Modifier.testTag(DevicesTags.CONFIRM_REVOKE),
                ) { Text("Revoke") }
            },
            dismissButton = { TextButton(onClick = { revoking = null }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun PairingCode(pairing: PairingUi) {
    if (pairing.expired) {
        Text(CODE_EXPIRED, Modifier.testTag(DevicesTags.COUNTDOWN))
    } else {
        Text(
            groupedCode(pairing.code),
            Modifier.testTag(DevicesTags.CODE),
            fontSize = 48.sp,
            fontFamily = FontFamily.Monospace,
        )
        Text("Expires in ${pairing.countdownText}", Modifier.testTag(DevicesTags.COUNTDOWN))
    }
    SelectionContainer {
        Column {
            Text("Server: ${pairing.serverName}")
            Text("Addresses: ${pairing.lanAddresses.joinToString(", ").ifEmpty { "none" }}")
            Text("Port: ${pairing.lanPort}")
            Text(
                "Fingerprint: ${groupedFingerprint(pairing.fingerprintPrefix)}",
                Modifier.testTag(DevicesTags.FINGERPRINT),
            )
        }
    }
}

@Composable
private fun DeviceRow(device: DeviceRowUi, onRevoke: () -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(device.name)
            Text(
                "${device.platformText} · paired ${device.pairedText} · last seen ${device.lastSeenText}",
                style = MaterialTheme.typography.bodySmall,
            )
        }
        TextButton(onClick = onRevoke, modifier = Modifier.testTag(DevicesTags.revoke(device.deviceId))) {
            Text("Revoke")
        }
    }
}
