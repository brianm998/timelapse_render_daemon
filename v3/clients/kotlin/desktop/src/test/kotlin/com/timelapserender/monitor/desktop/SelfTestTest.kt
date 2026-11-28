// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop

import com.timelapserender.monitor.core.fake.FakeTrdService
import io.grpc.Status
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File
import java.io.InputStream

/** Cannot load [missingClass] and does not find [missingResource]; everything else is [parent]'s. */
private class Incomplete(
    parent: ClassLoader,
    private val missingClass: String? = null,
    private val missingResource: String? = null,
) : ClassLoader(parent) {
    override fun loadClass(name: String, resolve: Boolean): Class<*> {
        if (name == missingClass) throw ClassNotFoundException(name)
        return super.loadClass(name, resolve)
    }

    override fun getResourceAsStream(name: String): InputStream? =
        if (name == missingResource) null else super.getResourceAsStream(name)
}

class SelfTestTest {
    @get:Rule
    val temp = TemporaryFolder()

    private val loader = SelfTestTest::class.java.classLoader
    private val printed = mutableListOf<String>()

    private fun run(vararg args: String, classLoader: ClassLoader = loader, tempDir: File = temp.root) =
        runSelfTest(args.toList(), classLoader, tempDir, print = { printed += it })

    private fun assertFailed(code: Int, vararg reasons: String) {
        assertEquals(1, code)
        val line = printed.single()
        assertTrue(line, line.startsWith("SELF-TEST FAILED: "))
        for (reason in reasons) assertTrue("$reason not in $line", reason in line)
    }

    @Test
    fun aCompleteRuntimePasses() {
        assertEquals(0, run("--self-test"))
        assertEquals(listOf("SELF-TEST OK"), printed)
        assertEquals("true", System.getProperty("java.awt.headless"))
        // The rendered screen is not left behind.
        assertEquals(emptyList<String>(), temp.root.list()!!.toList())
    }

    @Test
    fun aScreenClassThatCannotBeLoadedFails() {
        val missing = "com.timelapserender.monitor.desktop.ui.config.ConfigWindowKt"
        assertFailed(
            run("--self-test", classLoader = Incomplete(loader, missingClass = missing)),
            "loading the screen classes", "ClassNotFoundException", missing,
        )
    }

    @Test
    fun aMissingPosterFails() {
        assertFailed(
            run("--self-test", classLoader = Incomplete(loader, missingResource = "selftest/poster.jpg")),
            "decoding the sample poster", "selftest/poster.jpg is missing",
        )
    }

    @Test
    fun aScreenThatCannotBeWrittenFails() {
        assertFailed(run("--self-test", tempDir = File(temp.root, "absent")), "rendering the Activity screen")
    }

    @Test
    fun theSampleShowsEverySection() {
        val activity = sampleActivity()
        assertEquals(listOf("failure"), activity.failures.map { it.key })
        assertEquals(listOf("card"), activity.inProgress.map { it.key })
        assertEquals(1, activity.inProgress.single().variants.size)
        assertEquals(listOf("video"), activity.recent.map { it.key })
    }

    @Test
    fun theResultGoesToTheOutputFileInsteadOfStdout() {
        val file = temp.newFile("result.txt")
        assertEquals(0, run("--self-test", "--self-test-output", file.path))
        assertEquals("SELF-TEST OK\n", file.readText())

        val missing = SCREEN_CLASSES.first()
        val code = run(
            "--self-test-output", file.path, "--self-test",
            classLoader = Incomplete(loader, missingClass = missing),
        )
        assertEquals(1, code)
        assertTrue(file.readText().startsWith("SELF-TEST FAILED: loading the screen classes"))
        assertEquals(emptyList<String>(), printed)
    }

    @Test
    fun anOutputFileThatCannotBeWrittenFails() {
        val file = File(temp.root, "absent/result.txt")
        assertFailed(run("--self-test", "--self-test-output", file.path), "writing")
    }

    @Test
    fun theServerIsAskedForItsInfo() {
        val fake = FakeTrdService()
        fake.startPlaintext().use { server ->
            assertEquals(0, run("--self-test", "--server", "127.0.0.1:${server.port}"))
        }
        assertEquals(listOf("SELF-TEST OK"), printed)
        assertEquals(1, fake.callCount("GetServerInfo"))
    }

    @Test
    fun aServerThatRefusesFails() {
        val fake = FakeTrdService()
        fake.failWith["GetServerInfo"] = Status.INTERNAL.withDescription("broken\nbadly")
        fake.startPlaintext().use { server ->
            assertFailed(run("--self-test", "--server", "127.0.0.1:${server.port}"), "calling GetServerInfo", "INTERNAL")
        }
        assertTrue('\n' !in printed.single())
    }

    @Test
    fun aServerThatNeverAnswersFailsAtTheDeadline() {
        val fake = FakeTrdService()
        fake.gates["GetServerInfo"] = CompletableDeferred()
        fake.startPlaintext().use { server ->
            val args = listOf("--self-test", "--server", "127.0.0.1:${server.port}")
            val code = runSelfTest(args, tempDir = temp.root, serverTimeoutMs = 1, print = { printed += it })
            assertFailed(code, "calling GetServerInfo", "TimeoutCancellationException")
        }
    }

    @Test
    fun anUnreachableServerFails() {
        // A port that was just free: nothing listens on it once the fake has stopped.
        val port = FakeTrdService().startPlaintext().use { it.port }
        assertFailed(run("--self-test", "--server", "127.0.0.1:$port"), "calling GetServerInfo on 127.0.0.1:$port")
    }

    @Test
    fun badArgumentsFail() {
        for (args in listOf(
            listOf("--self-test", "--verbose"),
            listOf("--self-test", "--self-test-output"),
            listOf("--self-test", "--server"),
            listOf("--self-test", "--server", "10.0.0.2:50081"),
        )) {
            printed.clear()
            assertFailed(run(*args.toTypedArray()), "reading the command line")
        }
        val file = temp.newFile("bad.txt")
        assertEquals(1, run("--self-test", "--verbose", "--self-test-output", file.path))
        assertTrue(file.readText().startsWith("SELF-TEST FAILED: reading the command line"))
    }

    @Test
    fun theClassListCoversEveryUiSourceFile() {
        val root = File("src/main/kotlin")
        val sources = File(root, "com/timelapserender/monitor/desktop/ui").walk()
            .filter { it.extension == "kt" }
            .map { it.relativeTo(root).invariantSeparatorsPath.removeSuffix(".kt").replace('/', '.') + "Kt" }
            .toList()
        assertEquals(sources.sorted(), SCREEN_CLASSES.sorted())
    }
}
