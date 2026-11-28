// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.fake

import com.timelapserender.trd.v1.*
import io.grpc.ManagedChannel
import io.grpc.Metadata
import io.grpc.Server
import io.grpc.ServerCall
import io.grpc.ServerCallHandler
import io.grpc.ServerInterceptor
import io.grpc.ServerInterceptors
import io.grpc.ServerServiceDefinition
import io.grpc.Status
import io.grpc.inprocess.InProcessChannelBuilder
import io.grpc.inprocess.InProcessServerBuilder
import io.grpc.netty.shaded.io.grpc.netty.NettyServerBuilder
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.asFlow
import java.net.InetAddress
import java.net.InetSocketAddress
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.TimeUnit

/** One RPC that reached the fake: its bare method name and the `authorization` header it carried. */
data class RecordedCall(val method: String, val authorization: String?)

/**
 * An in-process implementation of every RPC of trd.proto, driven by scripted data. Every call is
 * recorded. Set [failWith] to make a method (by bare name, e.g. "GetServerInfo") fail with a status
 * before it reaches the handler. Start it with [startInProcess] or [startTls] (Netty, port 0).
 */
class FakeTrdService {
    val calls = CopyOnWriteArrayList<RecordedCall>()
    val failWith = ConcurrentHashMap<String, Status>()

    @Volatile var serverInfo: ServerInfo = ServerInfo.newBuilder()
        .setServerId("fake-server").setName("Fake trd").setVersion("3.0.0").setApiVersion(1)
        .setPlatform("linux-x64").setCallerIsLocal(true).build()
    @Volatile var pairResponse: PairResponse = PairResponse.newBuilder()
        .setToken("fake-token").setDeviceId("device-1").setServerId("fake-server").setServerName("Fake trd").build()
    @Volatile var snapshots: List<Snapshot> = listOf(Snapshot.newBuilder().setRevision(1).build())
    @Volatile var logEntries: List<LogEntry> = emptyList()
    @Volatile var configState: ConfigState = ConfigState.getDefaultInstance()
    @Volatile var renderedVideos: List<RenderedVideo> = emptyList()
    @Volatile var thumbnail: Thumbnail = Thumbnail.getDefaultInstance()
    @Volatile var renderDetails: RenderDetails = RenderDetails.getDefaultInstance()
    @Volatile var pairedDevices: List<PairedDevice> = emptyList()
    @Volatile var startPairingResponse: StartPairingResponse = StartPairingResponse.newBuilder().setCode("123456").build()
    @Volatile var directoryListing: ListDirectoryResponse = ListDirectoryResponse.getDefaultInstance()

    /** Per-id thumbnails; when empty every id gets [thumbnail], else an id not listed is NOT_FOUND. */
    @Volatile var thumbnails: Map<String, Thumbnail> = emptyMap()
    /** Per-path listings; when empty every path gets [directoryListing], else a path not listed is NOT_FOUND. */
    @Volatile var directories: Map<String, ListDirectoryResponse> = emptyMap()
    @Volatile var validateResponse: ValidateConfigResponse = ValidateConfigResponse.getDefaultInstance()
    /** What UpdateConfig / UpdateCodecs answer; [configState] when null. */
    @Volatile var updateConfigResult: ConfigState? = null
    @Volatile var updateCodecsResult: ConfigState? = null

    /** A handler of the named method (bare name) waits for this gate before it answers. */
    val gates = ConcurrentHashMap<String, CompletableDeferred<Unit>>()

    /** The request messages of Pair, the controls that name a target or a value, WatchSnapshots, GetRenderDetails, ListRenderedVideos, ListLog, WatchLog, ValidateConfig, UpdateConfig, UpdateCodecs, SetConfigPath and ListDirectory, in arrival order. */
    val requests = CopyOnWriteArrayList<Any>()

    inline fun <reified T : Any> requestsOf(): List<T> = requests.filterIsInstance<T>()

    fun callCount(method: String): Int = calls.count { it.method == method }

    fun methodsCalled(): Set<String> = calls.mapTo(HashSet()) { it.method }

    private val ack = Ack.getDefaultInstance()

    /** An entry passes when its level is at least [min]; UNSPECIFIED (really INFO) lets everything through in this fake. */
    private fun passesLevel(entry: LogEntry, min: LogLevel) =
        min == LogLevel.LOG_LEVEL_UNSPECIFIED || entry.level.number >= min.number

    private fun acked(request: Any): Ack {
        requests += request
        return ack
    }

    private val impl = object : TrdServiceGrpcKt.TrdServiceCoroutineImplBase() {
        override suspend fun getServerInfo(request: GetServerInfoRequest) = serverInfo
        override suspend fun pair(request: PairRequest): PairResponse {
            requests += request
            return pairResponse
        }
        override suspend fun getSnapshot(request: GetSnapshotRequest) = snapshots.last()
        override fun watchSnapshots(request: WatchSnapshotsRequest): Flow<Snapshot> {
            requests += request
            return snapshots.asFlow()
        }
        override suspend fun listRenderedVideos(request: ListRenderedVideosRequest): ListRenderedVideosResponse {
            requests += request
            val needle = request.search.lowercase()
            val all = renderedVideos.filter {
                needle.isEmpty() || it.sequenceName.lowercase().contains(needle) || it.outputFilename.lowercase().contains(needle)
            }
            val start = if (request.beforeId.isEmpty()) 0 else all.indexOfFirst { it.id == request.beforeId } + 1
            val limit = if (request.limit == 0) 100 else request.limit
            val page = all.drop(start).take(limit)
            return ListRenderedVideosResponse.newBuilder().addAllVideos(page).setMore(start + page.size < all.size).build()
        }
        override suspend fun getThumbnail(request: GetThumbnailRequest): Thumbnail {
            gates["GetThumbnail"]?.await()
            val map = thumbnails
            if (map.isEmpty()) return thumbnail
            return map[request.renderedVideoId] ?: throw Status.NOT_FOUND.withDescription("no thumbnail").asException()
        }
        override suspend fun getRenderDetails(request: GetRenderDetailsRequest): RenderDetails {
            requests += request
            return renderDetails
        }
        override suspend fun listLog(request: ListLogRequest): ListLogResponse {
            requests += request
            val all = logEntries.filter { passesLevel(it, request.minLevel) }.sortedByDescending { it.seq }
                .filter { request.beforeSeq == 0L || it.seq < request.beforeSeq }
            val limit = if (request.limit == 0) 200 else request.limit
            return ListLogResponse.newBuilder().addAllEntries(all.take(limit)).setMore(all.size > limit).build()
        }
        override fun watchLog(request: WatchLogRequest): Flow<LogEntry> {
            requests += request
            return logEntries.filter { it.seq > request.afterSeq && passesLevel(it, request.minLevel) }.asFlow()
        }
        override suspend fun setGlobalPause(request: SetGlobalPauseRequest) = acked(request)
        override suspend fun pauseSequence(request: SequenceRef) = acked(request)
        override suspend fun resumeSequence(request: SequenceRef) = acked(request)
        override suspend fun pauseVariant(request: VariantRef) = acked(request)
        override suspend fun resumeVariant(request: VariantRef) = acked(request)
        override suspend fun suspendRender(request: RenderRef) = acked(request)
        override suspend fun resumeRender(request: RenderRef) = acked(request)
        override suspend fun stopRender(request: RenderRef) = acked(request)
        override suspend fun retryVariant(request: VariantRef) = acked(request)
        override suspend fun dismissFailure(request: FailureRef) = acked(request)
        override suspend fun dismissAllFailures(request: DismissAllFailuresRequest) = ack
        override suspend fun rescanNow(request: RescanNowRequest): Ack {
            gates["RescanNow"]?.await()
            return ack
        }
        override suspend fun getConfig(request: GetConfigRequest) = configState
        override suspend fun validateConfig(request: ValidateConfigRequest): ValidateConfigResponse {
            requests += request
            return validateResponse
        }
        override suspend fun updateConfig(request: UpdateConfigRequest): ConfigState {
            requests += request
            return updateConfigResult ?: configState
        }
        override suspend fun updateCodecs(request: UpdateCodecsRequest): ConfigState {
            requests += request
            return updateCodecsResult ?: configState
        }
        override suspend fun setConfigPath(request: SetConfigPathRequest): ConfigState {
            requests += request
            return configState
        }
        override suspend fun listDirectory(request: ListDirectoryRequest): ListDirectoryResponse {
            requests += request
            val map = directories
            if (map.isEmpty()) return directoryListing
            return map[request.path] ?: throw Status.NOT_FOUND.withDescription("no such directory").asException()
        }
        override suspend fun startPairing(request: StartPairingRequest) = startPairingResponse
        override suspend fun listPairedDevices(request: ListPairedDevicesRequest) =
            ListPairedDevicesResponse.newBuilder().addAllDevices(pairedDevices).build()
        override suspend fun revokeDevice(request: RevokeDeviceRequest) = ack
    }

    private val recorder = object : ServerInterceptor {
        override fun <ReqT, RespT> interceptCall(
            call: ServerCall<ReqT, RespT>,
            headers: Metadata,
            next: ServerCallHandler<ReqT, RespT>,
        ): ServerCall.Listener<ReqT> {
            val method = call.methodDescriptor.bareMethodName.orEmpty()
            calls += RecordedCall(method, headers.get(AUTHORIZATION))
            val status = failWith[method]
            if (status != null) {
                call.close(status, Metadata())
                return object : ServerCall.Listener<ReqT>() {}
            }
            return next.startCall(call, headers)
        }
    }

    private fun definition(): ServerServiceDefinition = ServerInterceptors.intercept(impl, recorder)

    /** A running fake: [channel] talks to it; [close] stops it. */
    class Running internal constructor(
        private val server: Server,
        val channel: ManagedChannel?,
        val fingerprintSha256Hex: String?,
        private val inProcessName: String? = null,
    ) : AutoCloseable {
        private val extra = CopyOnWriteArrayList<ManagedChannel>()

        val port: Int get() = server.port

        /** Another channel to an in-process fake, for a client that shuts its own channel down. */
        fun newChannel(): ManagedChannel {
            val name = checkNotNull(inProcessName) { "Only an in-process fake hands out channels" }
            return InProcessChannelBuilder.forName(name).build().also { extra += it }
        }

        override fun close() {
            channel?.shutdownNow()
            extra.forEach { it.shutdownNow() }
            server.shutdownNow()
            server.awaitTermination(30, TimeUnit.SECONDS)
        }
    }

    fun startInProcess(): Running {
        val name = "trd-fake-" + UUID.randomUUID()
        val server = InProcessServerBuilder.forName(name).addService(definition()).build().start()
        return Running(server, InProcessChannelBuilder.forName(name).build(), null, name)
    }

    /** A TLS server on a loopback port chosen by the OS; connect with a pin of [Running.fingerprintSha256Hex]. */
    fun startTls(cert: TestCertificate): Running {
        val server = NettyServerBuilder.forAddress(InetSocketAddress(InetAddress.getLoopbackAddress(), 0))
            .useTransportSecurity(cert.certificatePem.byteInputStream(), cert.privateKeyPem.byteInputStream())
            .addService(definition())
            .build()
            .start()
        return Running(server, null, cert.fingerprintSha256Hex)
    }

    private companion object {
        val AUTHORIZATION: Metadata.Key<String> = Metadata.Key.of("authorization", Metadata.ASCII_STRING_MARSHALLER)
    }
}
