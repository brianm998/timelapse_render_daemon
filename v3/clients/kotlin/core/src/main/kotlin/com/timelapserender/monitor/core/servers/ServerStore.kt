// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.servers

import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths
import java.nio.file.StandardCopyOption
import java.nio.file.attribute.PosixFilePermission
import java.nio.file.attribute.PosixFilePermissions

data class ServerRecord(
    val serverId: String,
    val name: String,
    val host: String,
    val port: Int,
    val fingerprint: String,
    val token: String?,
    val platform: String,
    val lastSeenMs: Long,
)

interface ServerStore {
    fun list(): List<ServerRecord>

    /** Inserts the record, or replaces the one with the same server id. */
    fun save(record: ServerRecord)

    fun remove(serverId: String)
}

/** Stores `servers.json` under [dir]; the file is rw------- and the directory rwx------ where POSIX is available. */
class JsonFileServerStore(private val dir: Path) : ServerStore {
    private val file: Path = dir.resolve(FILE_NAME)
    private val lock = Any()

    override fun list(): List<ServerRecord> = synchronized(lock) { read() }

    override fun save(record: ServerRecord) = synchronized(lock) {
        write(read().filter { it.serverId != record.serverId } + record)
    }

    override fun remove(serverId: String) = synchronized(lock) {
        val current = read()
        val kept = current.filter { it.serverId != serverId }
        if (kept.size != current.size) write(kept)
    }

    private fun read(): List<ServerRecord> {
        if (!Files.isRegularFile(file)) return emptyList()
        return try {
            val root = MiniJson.parse(String(Files.readAllBytes(file), Charsets.UTF_8)) as? Map<*, *> ?: return emptyList()
            (root["servers"] as? List<*>).orEmpty().mapNotNull { decode(it) }
        } catch (_: Exception) {
            emptyList()
        }
    }

    private fun decode(value: Any?): ServerRecord? {
        val m = value as? Map<*, *> ?: return null
        return ServerRecord(
            serverId = m["serverId"] as? String ?: return null,
            name = m["name"] as? String ?: "",
            host = m["host"] as? String ?: return null,
            port = (m["port"] as? Number)?.toInt() ?: return null,
            fingerprint = m["fingerprint"] as? String ?: "",
            token = m["token"] as? String,
            platform = m["platform"] as? String ?: "",
            lastSeenMs = (m["lastSeenMs"] as? Number)?.toLong() ?: 0L,
        )
    }

    private fun write(records: List<ServerRecord>) {
        val posix = dir.fileSystem.supportedFileAttributeViews().contains("posix")
        if (posix) {
            Files.createDirectories(dir, PosixFilePermissions.asFileAttribute(DIR_PERMISSIONS))
            Files.setPosixFilePermissions(dir, DIR_PERMISSIONS)
        } else {
            Files.createDirectories(dir)
        }
        val tmp = if (posix) {
            Files.createTempFile(dir, "servers", ".tmp", PosixFilePermissions.asFileAttribute(FILE_PERMISSIONS))
        } else {
            Files.createTempFile(dir, "servers", ".tmp")
        }
        try {
            val json = MiniJson.write(
                mapOf(
                    "servers" to records.map {
                        linkedMapOf(
                            "serverId" to it.serverId, "name" to it.name, "host" to it.host, "port" to it.port,
                            "fingerprint" to it.fingerprint, "token" to it.token, "platform" to it.platform,
                            "lastSeenMs" to it.lastSeenMs,
                        )
                    },
                ),
            )
            Files.write(tmp, json.toByteArray(Charsets.UTF_8))
            try {
                Files.move(tmp, file, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
            } catch (_: AtomicMoveNotSupportedException) {
                Files.move(tmp, file, StandardCopyOption.REPLACE_EXISTING)
            }
        } finally {
            Files.deleteIfExists(tmp)
        }
    }

    companion object {
        const val FILE_NAME = "servers.json"
        private val FILE_PERMISSIONS = setOf(PosixFilePermission.OWNER_READ, PosixFilePermission.OWNER_WRITE)
        private val DIR_PERMISSIONS = FILE_PERMISSIONS + PosixFilePermission.OWNER_EXECUTE
    }
}

/**
 * The client's data directory (spec 2 §5.2): macOS `~/Library/Application Support/TimelapseRenderMonitor`,
 * Linux `$XDG_CONFIG_HOME/trm`, Windows `%APPDATA%\TimelapseRenderMonitor`. [osName] is `os.name`.
 */
fun defaultDataDir(osName: String, env: (String) -> String?, home: String): Path {
    val os = osName.lowercase()
    return when {
        os.contains("mac") || os.contains("darwin") ->
            Paths.get(home, "Library", "Application Support", "TimelapseRenderMonitor")
        os.contains("win") ->
            Paths.get(env("APPDATA")?.takeIf { it.isNotEmpty() } ?: Paths.get(home, "AppData", "Roaming").toString())
                .resolve("TimelapseRenderMonitor")
        else ->
            Paths.get(env("XDG_CONFIG_HOME")?.takeIf { it.isNotEmpty() } ?: Paths.get(home, ".config").toString())
                .resolve("trm")
    }
}

/** A small JSON reader and writer for [JsonFileServerStore]; values are Map, List, String, Number, Boolean or null. */
internal object MiniJson {
    fun write(value: Any?): String = StringBuilder().also { write(it, value) }.toString()

    private fun write(sb: StringBuilder, value: Any?) {
        when (value) {
            null -> sb.append("null")
            is String -> quote(sb, value)
            is Number, is Boolean -> sb.append(value.toString())
            is Map<*, *> -> {
                sb.append('{')
                value.entries.forEachIndexed { i, e ->
                    if (i > 0) sb.append(',')
                    quote(sb, e.key.toString())
                    sb.append(':')
                    write(sb, e.value)
                }
                sb.append('}')
            }
            is List<*> -> {
                sb.append('[')
                value.forEachIndexed { i, v ->
                    if (i > 0) sb.append(',')
                    write(sb, v)
                }
                sb.append(']')
            }
            else -> throw IllegalArgumentException("Unsupported JSON value: ${value::class}")
        }
    }

    private fun quote(sb: StringBuilder, s: String) {
        sb.append('"')
        for (c in s) {
            when {
                c == '"' -> sb.append("\\\"")
                c == '\\' -> sb.append("\\\\")
                c == '\n' -> sb.append("\\n")
                c == '\r' -> sb.append("\\r")
                c == '\t' -> sb.append("\\t")
                c < ' ' -> sb.append("\\u%04x".format(c.code))
                else -> sb.append(c)
            }
        }
        sb.append('"')
    }

    fun parse(text: String): Any? {
        val p = Parser(text)
        val v = p.value()
        p.skipWs()
        require(p.atEnd()) { "Trailing characters" }
        return v
    }

    private class Parser(private val s: String) {
        private var i = 0

        fun atEnd() = i >= s.length

        fun skipWs() {
            while (i < s.length && s[i].isWhitespace()) i++
        }

        fun value(): Any? {
            skipWs()
            require(i < s.length) { "Unexpected end" }
            return when (val c = s[i]) {
                '{' -> obj()
                '[' -> arr()
                '"' -> str()
                't' -> literal("true", true)
                'f' -> literal("false", false)
                'n' -> literal("null", null)
                else -> if (c == '-' || c.isDigit()) num() else throw IllegalArgumentException("Unexpected '$c'")
            }
        }

        private fun literal(word: String, v: Any?): Any? {
            require(s.startsWith(word, i)) { "Bad literal" }
            i += word.length
            return v
        }

        private fun num(): Number {
            val start = i
            while (i < s.length && (s[i].isDigit() || s[i] in "+-.eE")) i++
            val t = s.substring(start, i)
            return t.toLongOrNull() ?: t.toDouble()
        }

        private fun str(): String {
            i++
            val sb = StringBuilder()
            while (true) {
                require(i < s.length) { "Unterminated string" }
                val c = s[i++]
                when (c) {
                    '"' -> return sb.toString()
                    '\\' -> {
                        require(i < s.length) { "Bad escape" }
                        when (val e = s[i++]) {
                            'n' -> sb.append('\n')
                            'r' -> sb.append('\r')
                            't' -> sb.append('\t')
                            'b' -> sb.append('\b')
                            'f' -> sb.append('\u000c')
                            'u' -> {
                                require(i + 4 <= s.length) { "Bad escape" }
                                sb.append(s.substring(i, i + 4).toInt(16).toChar())
                                i += 4
                            }
                            '"', '\\', '/' -> sb.append(e)
                            else -> throw IllegalArgumentException("Bad escape")
                        }
                    }
                    else -> sb.append(c)
                }
            }
        }

        private fun arr(): List<Any?> {
            i++
            val out = ArrayList<Any?>()
            skipWs()
            if (i < s.length && s[i] == ']') {
                i++
                return out
            }
            while (true) {
                out.add(value())
                skipWs()
                require(i < s.length) { "Unterminated array" }
                when (s[i++]) {
                    ',' -> Unit
                    ']' -> return out
                    else -> throw IllegalArgumentException("Bad array")
                }
            }
        }

        private fun obj(): Map<String, Any?> {
            i++
            val out = LinkedHashMap<String, Any?>()
            skipWs()
            if (i < s.length && s[i] == '}') {
                i++
                return out
            }
            while (true) {
                skipWs()
                require(i < s.length && s[i] == '"') { "Bad object key" }
                val k = str()
                skipWs()
                require(i < s.length && s[i++] == ':') { "Bad object" }
                out[k] = value()
                skipWs()
                require(i < s.length) { "Unterminated object" }
                when (s[i++]) {
                    ',' -> Unit
                    '}' -> return out
                    else -> throw IllegalArgumentException("Bad object")
                }
            }
        }
    }
}
