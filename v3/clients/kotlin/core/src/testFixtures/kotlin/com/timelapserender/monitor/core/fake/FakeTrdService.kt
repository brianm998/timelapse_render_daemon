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

    fun callCount(method: String): Int = calls.count { it.method == method }

    fun methodsCalled(): Set<String> = calls.mapTo(HashSet()) { it.method }

    private val ack = Ack.getDefaultInstance()

    private val impl = object : TrdServiceGrpcKt.TrdServiceCoroutineImplBase() {
        override suspend fun getServerInfo(request: GetServerInfoRequest) = serverInfo
        override suspend fun pair(request: PairRequest) = pairResponse
        override suspend fun getSnapshot(request: GetSnapshotRequest) = snapshots.last()
        override fun watchSnapshots(request: WatchSnapshotsRequest): Flow<Snapshot> = snapshots.asFlow()
        override suspend fun listRenderedVideos(request: ListRenderedVideosRequest) =
            ListRenderedVideosResponse.newBuilder().addAllVideos(renderedVideos).build()
        override suspend fun getThumbnail(request: GetThumbnailRequest) = thumbnail
        override suspend fun getRenderDetails(request: GetRenderDetailsRequest) = renderDetails
        override suspend fun listLog(request: ListLogRequest) =
            ListLogResponse.newBuilder().addAllEntries(logEntries.reversed()).build()
        override fun watchLog(request: WatchLogRequest): Flow<LogEntry> =
            logEntries.filter { it.seq > request.afterSeq }.asFlow()
        override suspend fun setGlobalPause(request: SetGlobalPauseRequest) = ack
        override suspend fun pauseSequence(request: SequenceRef) = ack
        override suspend fun resumeSequence(request: SequenceRef) = ack
        override suspend fun pauseVariant(request: VariantRef) = ack
        override suspend fun resumeVariant(request: VariantRef) = ack
        override suspend fun suspendRender(request: RenderRef) = ack
        override suspend fun resumeRender(request: RenderRef) = ack
        override suspend fun stopRender(request: RenderRef) = ack
        override suspend fun retryVariant(request: VariantRef) = ack
        override suspend fun dismissFailure(request: FailureRef) = ack
        override suspend fun dismissAllFailures(request: DismissAllFailuresRequest) = ack
        override suspend fun rescanNow(request: RescanNowRequest) = ack
        override suspend fun getConfig(request: GetConfigRequest) = configState
        override suspend fun validateConfig(request: ValidateConfigRequest) = ValidateConfigResponse.getDefaultInstance()
        override suspend fun updateConfig(request: UpdateConfigRequest) = configState
        override suspend fun updateCodecs(request: UpdateCodecsRequest) = configState
        override suspend fun setConfigPath(request: SetConfigPathRequest) = configState
        override suspend fun listDirectory(request: ListDirectoryRequest) = directoryListing
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
    ) : AutoCloseable {
        val port: Int get() = server.port

        override fun close() {
            channel?.shutdownNow()
            server.shutdownNow()
            server.awaitTermination(30, TimeUnit.SECONDS)
        }
    }

    fun startInProcess(): Running {
        val name = "trd-fake-" + UUID.randomUUID()
        val server = InProcessServerBuilder.forName(name).addService(definition()).build().start()
        return Running(server, InProcessChannelBuilder.forName(name).build(), null)
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
