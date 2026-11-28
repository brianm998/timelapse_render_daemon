// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.net

import com.timelapserender.monitor.core.fake.FakeTrdService
import com.timelapserender.trd.v1.Config
import com.timelapserender.trd.v1.DevicePlatform
import com.timelapserender.trd.v1.GetRenderDetailsRequest
import com.timelapserender.trd.v1.ServerInfo
import io.grpc.Status
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.CopyOnWriteArrayList

class TrdClientTest {
    private val fake = FakeTrdService()
    private val running = fake.startInProcess()
    private val scope = CoroutineScope(Dispatchers.Default + Job())

    @After
    fun tearDown() {
        scope.cancel()
        running.close()
    }

    private fun client(token: String? = null, lan: Boolean = false, delayer: suspend (Long) -> Unit = {}) =
        TrdClient(running.channel!!, token, scope, lan = lan, clock = { 42L }, delayer = delayer)

    private suspend fun TrdClient.awaitState(predicate: (ConnectionState) -> Boolean): ConnectionState =
        withTimeout(60_000) { state.first(predicate) }

    @Test
    fun connectsAndReportsServerInfo() = runBlocking {
        val c = client()
        c.start()
        val s = c.awaitState { it is ConnectionState.Connected } as ConnectionState.Connected
        assertEquals("fake-server", s.info.serverId)
    }

    @Test
    fun unauthenticatedNeedsPairing() = runBlocking {
        fake.failWith["GetServerInfo"] = Status.UNAUTHENTICATED
        val c = client()
        c.start()
        assertEquals(ConnectionState.NeedsPairing, c.awaitState { it is ConnectionState.NeedsPairing })
    }

    @Test
    fun unauthenticatedFromACallNeedsPairing() = runBlocking {
        val c = client()
        c.start()
        c.awaitState { it is ConnectionState.Connected }
        fake.failWith["GetSnapshot"] = Status.UNAUTHENTICATED
        try {
            c.getSnapshot()
        } catch (_: Exception) {
        }
        assertEquals(ConnectionState.NeedsPairing, c.state.value)
    }

    @Test
    fun lanCallerThatIsNeitherLocalNorPairedNeedsPairing() = runBlocking {
        fake.serverInfo = fake.serverInfo.toBuilder().setCallerIsLocal(false).setCallerIsPaired(false).build()
        val c = client(lan = true)
        c.start()
        assertEquals(ConnectionState.NeedsPairing, c.awaitState { it is ConnectionState.NeedsPairing })
    }

    @Test
    fun pairedLanCallerConnects() = runBlocking {
        fake.serverInfo = fake.serverInfo.toBuilder().setCallerIsLocal(false).setCallerIsPaired(true).build()
        val c = client(token = "t", lan = true)
        c.start()
        c.awaitState { it is ConnectionState.Connected }
        Unit
    }

    @Test
    fun apiVersion2IsIncompatible() = runBlocking {
        fake.serverInfo = ServerInfo.newBuilder().setApiVersion(2).build()
        val c = client()
        c.start()
        val s = c.awaitState { it is ConnectionState.Incompatible } as ConnectionState.Incompatible
        assertEquals(2, s.serverVersion)
        assertEquals("This trd uses API version 2; this app supports version 1.", s.message)
        assertEquals(1, fake.callCount("GetServerInfo"))
    }

    @Test
    fun backOffIsOneTwoFiveTenSecondsThenOffline() = runBlocking {
        fake.failWith["GetServerInfo"] = Status.UNAVAILABLE
        val delays = CopyOnWriteArrayList<Long>()
        val fourth = CompletableDeferred<Unit>()
        val c = client { ms ->
            delays += ms
            if (delays.size == 4) fourth.complete(Unit)
            if (delays.size >= 4) awaitCancellation()
        }
        c.start()
        withTimeout(60_000) { fourth.await() }
        assertEquals(listOf(1000L, 2000L, 5000L, 10_000L), delays.toList())
        assertEquals(ConnectionState.Offline(0L), c.state.value)
    }

    @Test
    fun reconnectingCountsAttemptsBeforeOffline() = runBlocking {
        fake.failWith["GetServerInfo"] = Status.UNAVAILABLE
        val seen = CopyOnWriteArrayList<ConnectionState>()
        val gate = CompletableDeferred<Unit>()
        lateinit var c: TrdClient
        c = client { _ ->
            seen += c.state.value
            if (seen.size == 4) gate.complete(Unit)
            if (seen.size >= 4) awaitCancellation()
        }
        c.start()
        withTimeout(60_000) { gate.await() }
        assertEquals(
            listOf(
                ConnectionState.Reconnecting(1, 0L),
                ConnectionState.Reconnecting(2, 0L),
                ConnectionState.Reconnecting(3, 0L),
                ConnectionState.Offline(0L),
            ),
            seen.toList(),
        )
    }

    @Test
    fun retryingRecoversWhenTheServerComesBack() = runBlocking {
        fake.failWith["GetServerInfo"] = Status.UNAVAILABLE
        val c = client { _ -> fake.failWith.remove("GetServerInfo") }
        c.start()
        c.awaitState { it is ConnectionState.Connected }
        Unit
    }

    @Test
    fun tokenInterceptorAddsBearerOnlyWhenATokenIsSet() = runBlocking {
        val withToken = client(token = "abc")
        withToken.serverInfo()
        assertEquals("Bearer abc", fake.calls.last().authorization)
        val without = client(token = null)
        without.serverInfo()
        assertNull(fake.calls.last().authorization)
        val empty = client(token = "")
        empty.serverInfo()
        assertNull(fake.calls.last().authorization)
    }

    @Test
    fun everyRpcReachesTheFake() = runBlocking {
        val c = client(token = "t")
        c.serverInfo()
        c.pair("123456", "Desk", DevicePlatform.DEVICE_PLATFORM_DESKTOP)
        c.getSnapshot()
        c.watchSnapshots(2).first()
        c.listRenderedVideos(10, "", "x")
        c.getThumbnail("v1")
        c.getRenderDetails(GetRenderDetailsRequest.newBuilder().setRenderId("r1").build())
        c.listLog(0, 10)
        fake.logEntries = listOf(com.timelapserender.trd.v1.LogEntry.newBuilder().setSeq(5).build())
        assertEquals(5L, c.watchLog(0).first().seq)
        c.setGlobalPause(true)
        c.pauseSequence("s")
        c.resumeSequence("s")
        c.pauseVariant("s", "k")
        c.resumeVariant("s", "k")
        c.suspendRender("r")
        c.resumeRender("r")
        c.stopRender("r")
        c.retryVariant("s", "k")
        c.dismissFailure("f")
        c.dismissAllFailures()
        c.rescanNow()
        c.getConfig()
        c.validateConfig(Config.getDefaultInstance())
        c.updateConfig(Config.getDefaultInstance(), "rev")
        c.updateCodecs(emptyList(), "rev")
        c.setConfigPath("/x", true)
        c.listDirectory("/")
        c.startPairing()
        c.listPairedDevices()
        c.revokeDevice("d")
        val expected = setOf(
            "GetServerInfo", "Pair", "GetSnapshot", "WatchSnapshots", "ListRenderedVideos", "GetThumbnail",
            "GetRenderDetails", "ListLog", "WatchLog", "SetGlobalPause", "PauseSequence", "ResumeSequence",
            "PauseVariant", "ResumeVariant", "SuspendRender", "ResumeRender", "StopRender", "RetryVariant",
            "DismissFailure", "DismissAllFailures", "RescanNow", "GetConfig", "ValidateConfig", "UpdateConfig",
            "UpdateCodecs", "SetConfigPath", "ListDirectory", "StartPairing", "ListPairedDevices", "RevokeDevice",
        )
        assertEquals(expected, fake.methodsCalled())
        assertTrue(fake.calls.all { it.authorization == "Bearer t" })
    }
}
