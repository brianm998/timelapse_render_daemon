// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.monitor.core.format.Format
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.trd.v1.DevicePlatform
import com.timelapserender.trd.v1.StartPairingResponse
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.time.ZoneId

data class PairingUi(
    val code: String,
    val serverName: String,
    val lanAddresses: List<String>,
    val lanPort: Int,
    /** The first 16 hex digits of the certificate fingerprint. */
    val fingerprintPrefix: String,
    val expiresAtMs: Long,
    val remainingSeconds: Long,
    val countdownText: String,
) {
    val expired: Boolean get() = remainingSeconds <= 0
}

data class DeviceRowUi(
    val deviceId: String,
    val name: String,
    val platformText: String,
    val pairedText: String,
    val lastSeenText: String,
)

data class DevicesUi(
    val pairing: PairingUi? = null,
    val devices: List<DeviceRowUi> = emptyList(),
    val error: String? = null,
)

/** Desktop over loopback only: `StartPairing` with a countdown, `ListPairedDevices`, `RevokeDevice`. */
class DevicesModel(
    private val client: TrdClient,
    private val scope: CoroutineScope,
    private val clock: () -> Long = System::currentTimeMillis,
    private val zone: ZoneId = ZoneId.systemDefault(),
    private val delayer: suspend (Long) -> Unit = { delay(it) },
) {
    private val _state = MutableStateFlow(DevicesUi())
    val state: StateFlow<DevicesUi> = _state.asStateFlow()
    private var countdown: Job? = null

    fun startPairing() {
        scope.launch {
            try {
                val r = client.startPairing()
                countdown?.cancel()
                _state.update { it.copy(error = null) }
                countdown = scope.launch { tick(r) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(error = e.statusMessage()) }
            }
        }
    }

    fun cancelPairing() {
        countdown?.cancel()
        _state.update { it.copy(pairing = null) }
    }

    fun refresh() {
        scope.launch { load() }
    }

    fun revoke(deviceId: String) {
        scope.launch {
            try {
                client.revokeDevice(deviceId)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(error = e.statusMessage()) }
                return@launch
            }
            load()
        }
    }

    private suspend fun tick(r: StartPairingResponse) {
        while (true) {
            val remainingMs = r.expiresAtMs - clock()
            val seconds = if (remainingMs <= 0) 0L else (remainingMs + 999) / 1000
            val ui = PairingUi(
                r.code, r.serverName, r.lanAddressesList, r.lanPort, r.certFingerprintSha256.take(16),
                r.expiresAtMs, seconds, Format.countdown(seconds),
            )
            _state.update { it.copy(pairing = ui) }
            if (seconds <= 0) return
            delayer(1000)
        }
    }

    private suspend fun load() {
        try {
            val now = clock()
            val rows = client.listPairedDevices().devicesList.map {
                DeviceRowUi(
                    it.deviceId, it.name, platformText(it.platform),
                    Format.absoluteTime(it.pairedAtMs, now, zone),
                    if (it.lastSeenMs <= 0) "never" else Format.relativeTime(it.lastSeenMs, now, zone),
                )
            }
            _state.update { it.copy(devices = rows, error = null) }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            _state.update { it.copy(error = e.statusMessage()) }
        }
    }

    private fun platformText(p: DevicePlatform) = when (p) {
        DevicePlatform.DEVICE_PLATFORM_IOS -> "iOS"
        DevicePlatform.DEVICE_PLATFORM_ANDROID -> "Android"
        DevicePlatform.DEVICE_PLATFORM_DESKTOP -> "Desktop"
        else -> "Unknown"
    }
}
