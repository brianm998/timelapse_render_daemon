// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.net

import com.timelapserender.trd.v1.*
import io.grpc.ManagedChannel
import io.grpc.Status
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.launch
import java.util.concurrent.TimeUnit

/**
 * A client for one trd. [start] runs the connect loop that drives [state]; every RPC of spec 1 §7
 * is a method. [lan] is true for a pinned-TLS endpoint: GetServerInfo is open there, so a caller
 * that is neither local nor paired still needs pairing.
 *
 * The back-off between failed connect attempts is [BACKOFF_MS] and is taken through [delayer] and
 * [clock] so tests inject their own.
 */
class TrdClient(
    private val channel: ManagedChannel,
    token: String?,
    private val scope: CoroutineScope,
    private val lan: Boolean = false,
    private val clock: () -> Long = System::currentTimeMillis,
    private val delayer: suspend (Long) -> Unit = { delay(it) },
) {
    private val stub = TrdServiceGrpcKt.TrdServiceCoroutineStub(channel).withInterceptors(TokenInterceptor(token))
    private val _state = MutableStateFlow<ConnectionState>(ConnectionState.Connecting)
    private val lock = Any()
    private var job: Job? = null

    @Volatile
    private var lastContactMs = 0L

    val state: StateFlow<ConnectionState> = _state.asStateFlow()

    /** Starts the connect loop unless it is already running (also how a stopped loop is retried). */
    fun start() {
        synchronized(lock) {
            if (job?.isActive == true) return
            job = scope.launch { connectLoop() }
        }
    }

    fun close() {
        synchronized(lock) { job?.cancel() }
        channel.shutdownNow()
    }

    private suspend fun connectLoop() {
        var failures = 0
        _state.value = ConnectionState.Connecting
        while (true) {
            try {
                val info = stub.getServerInfo(GetServerInfoRequest.getDefaultInstance())
                lastContactMs = clock()
                if (info.apiVersion != SUPPORTED_API_VERSION) {
                    _state.value = ConnectionState.Incompatible(info.apiVersion)
                } else if (lan && !info.callerIsLocal && !info.callerIsPaired) {
                    _state.value = ConnectionState.NeedsPairing
                } else {
                    _state.value = ConnectionState.Connected(info)
                }
                return
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (Status.fromThrowable(e).code == Status.Code.UNAUTHENTICATED) {
                    _state.value = ConnectionState.NeedsPairing
                    return
                }
            }
            failures++
            _state.value = if (failures < OFFLINE_AFTER) {
                ConnectionState.Reconnecting(failures, lastContactMs)
            } else {
                ConnectionState.Offline(lastContactMs)
            }
            delayer(BACKOFF_MS[minOf(failures, BACKOFF_MS.size) - 1])
        }
    }

    private fun noteFailure(e: Throwable) {
        when (Status.fromThrowable(e).code) {
            Status.Code.UNAUTHENTICATED -> _state.value = ConnectionState.NeedsPairing
            Status.Code.UNAVAILABLE -> if (_state.value is ConnectionState.Connected) start()
            else -> Unit
        }
    }

    private suspend fun <T> call(block: suspend (TrdServiceGrpcKt.TrdServiceCoroutineStub) -> T): T =
        try {
            block(stub).also { lastContactMs = clock() }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            noteFailure(e)
            throw e
        }

    private fun <T> Flow<T>.noting(): Flow<T> = catch { e ->
        if (e !is CancellationException) noteFailure(e)
        throw e
    }

    // Server info and pairing.
    suspend fun serverInfo(): ServerInfo = call { it.getServerInfo(GetServerInfoRequest.getDefaultInstance()) }

    suspend fun pair(code: String, deviceName: String, platform: DevicePlatform): PairResponse =
        call {
            it.pair(PairRequest.newBuilder().setCode(code).setDeviceName(deviceName).setPlatform(platform).build())
        }

    suspend fun startPairing(): StartPairingResponse = call { it.startPairing(StartPairingRequest.getDefaultInstance()) }

    suspend fun listPairedDevices(): ListPairedDevicesResponse =
        call { it.listPairedDevices(ListPairedDevicesRequest.getDefaultInstance()) }

    suspend fun revokeDevice(deviceId: String) {
        call { it.revokeDevice(RevokeDeviceRequest.newBuilder().setDeviceId(deviceId).build()) }
    }

    // State.
    suspend fun getSnapshot(): Snapshot = call { it.getSnapshot(GetSnapshotRequest.getDefaultInstance()) }

    fun watchSnapshots(maxPerSecond: Int = 0): Flow<Snapshot> =
        stub.watchSnapshots(WatchSnapshotsRequest.newBuilder().setMaxPerSecond(maxPerSecond).build()).noting()

    suspend fun listRenderedVideos(limit: Int = 0, beforeId: String = "", search: String = ""): ListRenderedVideosResponse =
        call {
            it.listRenderedVideos(
                ListRenderedVideosRequest.newBuilder().setLimit(limit).setBeforeId(beforeId).setSearch(search).build(),
            )
        }

    suspend fun getThumbnail(renderedVideoId: String): Thumbnail =
        call { it.getThumbnail(GetThumbnailRequest.newBuilder().setRenderedVideoId(renderedVideoId).build()) }

    suspend fun getRenderDetails(request: GetRenderDetailsRequest): RenderDetails =
        call { it.getRenderDetails(request) }

    suspend fun listLog(beforeSeq: Long = 0, limit: Int = 0, minLevel: LogLevel = LogLevel.LOG_LEVEL_UNSPECIFIED): ListLogResponse =
        call {
            it.listLog(ListLogRequest.newBuilder().setBeforeSeq(beforeSeq).setLimit(limit).setMinLevel(minLevel).build())
        }

    fun watchLog(afterSeq: Long, minLevel: LogLevel = LogLevel.LOG_LEVEL_UNSPECIFIED): Flow<LogEntry> =
        stub.watchLog(WatchLogRequest.newBuilder().setAfterSeq(afterSeq).setMinLevel(minLevel).build()).noting()

    // Controls.
    suspend fun setGlobalPause(paused: Boolean) {
        call { it.setGlobalPause(SetGlobalPauseRequest.newBuilder().setPaused(paused).build()) }
    }

    suspend fun pauseSequence(sequenceName: String) {
        call { it.pauseSequence(sequenceRef(sequenceName)) }
    }

    suspend fun resumeSequence(sequenceName: String) {
        call { it.resumeSequence(sequenceRef(sequenceName)) }
    }

    suspend fun pauseVariant(sequenceName: String, variantKey: String) {
        call { it.pauseVariant(variantRef(sequenceName, variantKey)) }
    }

    suspend fun resumeVariant(sequenceName: String, variantKey: String) {
        call { it.resumeVariant(variantRef(sequenceName, variantKey)) }
    }

    suspend fun suspendRender(renderId: String) {
        call { it.suspendRender(renderRef(renderId)) }
    }

    suspend fun resumeRender(renderId: String) {
        call { it.resumeRender(renderRef(renderId)) }
    }

    suspend fun stopRender(renderId: String) {
        call { it.stopRender(renderRef(renderId)) }
    }

    suspend fun retryVariant(sequenceName: String, variantKey: String) {
        call { it.retryVariant(variantRef(sequenceName, variantKey)) }
    }

    suspend fun dismissFailure(failureId: String) {
        call { it.dismissFailure(FailureRef.newBuilder().setFailureId(failureId).build()) }
    }

    suspend fun dismissAllFailures() {
        call { it.dismissAllFailures(DismissAllFailuresRequest.getDefaultInstance()) }
    }

    suspend fun rescanNow() {
        call { it.rescanNow(RescanNowRequest.getDefaultInstance()) }
    }

    // Configuration.
    suspend fun getConfig(): ConfigState = call { it.getConfig(GetConfigRequest.getDefaultInstance()) }

    suspend fun validateConfig(config: Config, codecs: List<VideoCodec> = emptyList()): ValidateConfigResponse =
        call {
            it.validateConfig(ValidateConfigRequest.newBuilder().setConfig(config).addAllCodecs(codecs).build())
        }

    suspend fun updateConfig(config: Config, expectedRevision: String): ConfigState =
        call {
            it.updateConfig(
                UpdateConfigRequest.newBuilder().setConfig(config).setExpectedRevision(expectedRevision).build(),
            )
        }

    suspend fun updateCodecs(codecs: List<VideoCodec>, expectedRevision: String): ConfigState =
        call {
            it.updateCodecs(
                UpdateCodecsRequest.newBuilder().addAllCodecs(codecs).setExpectedRevision(expectedRevision).build(),
            )
        }

    suspend fun setConfigPath(path: String, createIfMissing: Boolean): ConfigState =
        call {
            it.setConfigPath(SetConfigPathRequest.newBuilder().setPath(path).setCreateIfMissing(createIfMissing).build())
        }

    suspend fun listDirectory(path: String, includeFiles: Boolean = false, showHidden: Boolean = false): ListDirectoryResponse =
        call {
            it.listDirectory(
                ListDirectoryRequest.newBuilder().setPath(path).setIncludeFiles(includeFiles).setShowHidden(showHidden).build(),
            )
        }

    private fun sequenceRef(name: String) = SequenceRef.newBuilder().setSequenceName(name).build()

    private fun variantRef(name: String, key: String) =
        VariantRef.newBuilder().setSequenceName(name).setVariantKey(key).build()

    private fun renderRef(id: String) = RenderRef.newBuilder().setRenderId(id).build()

    companion object {
        /** Delay after the 1st, 2nd, 3rd and every later failed attempt. */
        val BACKOFF_MS: List<Long> = listOf(1, 2, 5, 10).map { TimeUnit.SECONDS.toMillis(it.toLong()) }
        const val OFFLINE_AFTER = 4

        fun create(
            endpoint: ServerEndpoint,
            token: String?,
            scope: CoroutineScope,
            clock: () -> Long = System::currentTimeMillis,
            delayer: suspend (Long) -> Unit = { delay(it) },
        ): TrdClient = TrdClient(
            ChannelFactory.create(endpoint), token, scope,
            lan = endpoint.tls is TlsMode.Pinned, clock = clock, delayer = delayer,
        )
    }
}
