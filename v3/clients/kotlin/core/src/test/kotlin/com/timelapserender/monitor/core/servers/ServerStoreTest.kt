// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.servers

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.attribute.PosixFilePermissions

class ServerStoreTest {
    @get:Rule
    val tmp = TemporaryFolder()

    private val a = ServerRecord("a", "Studio \"A\"", "10.0.0.2", 50072, "ab".repeat(32), "tok\n\u00e9", "linux-x64", 123L)
    private val b = ServerRecord("b", "B", "10.0.0.3", 50072, "cd".repeat(32), null, "macos-universal", 0L)

    private fun dir(): Path = tmp.root.toPath().resolve("data")

    @Test
    fun serversJsonRoundTrips() {
        JsonFileServerStore(dir()).apply { save(a); save(b) }
        val reread = JsonFileServerStore(dir())
        assertEquals(listOf(a, b), reread.list())
        reread.save(a.copy(name = "Renamed"))
        assertEquals(listOf(b, a.copy(name = "Renamed")), JsonFileServerStore(dir()).list())
        reread.remove("b")
        assertEquals(listOf(a.copy(name = "Renamed")), JsonFileServerStore(dir()).list())
    }

    @Test
    fun filesAreOwnerOnlyOnPosix() {
        val d = dir()
        assumeTrue(d.fileSystem.supportedFileAttributeViews().contains("posix"))
        JsonFileServerStore(d).save(a)
        val file = d.resolve("servers.json")
        assertEquals("rw-------", PosixFilePermissions.toString(Files.getPosixFilePermissions(file)))
        assertEquals("rwx------", PosixFilePermissions.toString(Files.getPosixFilePermissions(d)))
        JsonFileServerStore(d).save(b)
        assertEquals("rw-------", PosixFilePermissions.toString(Files.getPosixFilePermissions(file)))
        assertEquals(listOf("servers.json"), Files.list(d).use { s -> s.map { it.fileName.toString() }.toList() })
    }

    @Test
    fun missingOrCorruptFileIsEmpty() {
        val d = dir()
        assertTrue(JsonFileServerStore(d).list().isEmpty())
        Files.createDirectories(d)
        Files.write(d.resolve("servers.json"), "{not json".toByteArray())
        val store = JsonFileServerStore(d)
        assertTrue(store.list().isEmpty())
        store.save(a)
        assertEquals(listOf(a), store.list())
    }

    @Test
    fun defaultDataDirFollowsThePlatform() {
        val env = mapOf("XDG_CONFIG_HOME" to "/xdg", "APPDATA" to "C:\\Users\\u\\AppData\\Roaming")
        assertEquals(
            "/Users/u/Library/Application Support/TimelapseRenderMonitor",
            defaultDataDir("Mac OS X", env::get, "/Users/u").toString(),
        )
        assertEquals("/xdg/trm", defaultDataDir("Linux", env::get, "/home/u").toString())
        assertEquals("/home/u/.config/trm", defaultDataDir("Linux", { null }, "/home/u").toString())
        val win = defaultDataDir("Windows 11", env::get, "C:\\Users\\u").toString()
        assertTrue(win, win.endsWith("TimelapseRenderMonitor") && win.contains("AppData"))
    }
}
