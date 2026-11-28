// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop

import com.timelapserender.monitor.core.state.LastSeqStore
import java.util.concurrent.ConcurrentHashMap
import java.util.prefs.Preferences

/** The snapshot rate (snapshots per second) the user may choose, and its default (spec 2 §6.8). */
val SNAPSHOT_RATES = 1..10
const val DEFAULT_SNAPSHOT_RATE = 4

/** The choices of the preferences window (spec 2 §6.8), and the last event seen per server. */
interface AppPreferences : LastSeqStore {
    var notifyFinished: Boolean
    var notifyFailed: Boolean
    var keepRunningInTray: Boolean

    /** Snapshots per second, always within [SNAPSHOT_RATES]. */
    var snapshotRate: Int

    /** "Don't ask again" of the Stop confirmation. */
    var skipStopConfirmation: Boolean

    /** "Don't ask again" of the Pause all rendering confirmation. */
    var skipPauseAllConfirmation: Boolean

    /** Asks every confirmation again. */
    fun resetConfirmations() {
        skipStopConfirmation = false
        skipPauseAllConfirmation = false
    }
}

/** [AppPreferences] in a `java.util.prefs` node (`com/timelapserender/monitor`, spec 2 §6.8). */
class JavaAppPreferences(private val node: Preferences) : AppPreferences {
    override var notifyFinished: Boolean
        get() = node.getBoolean(NOTIFY_FINISHED, true)
        set(value) = node.putBoolean(NOTIFY_FINISHED, value)

    override var notifyFailed: Boolean
        get() = node.getBoolean(NOTIFY_FAILED, true)
        set(value) = node.putBoolean(NOTIFY_FAILED, value)

    override var keepRunningInTray: Boolean
        get() = node.getBoolean(KEEP_IN_TRAY, true)
        set(value) = node.putBoolean(KEEP_IN_TRAY, value)

    override var snapshotRate: Int
        get() = node.getInt(SNAPSHOT_RATE, DEFAULT_SNAPSHOT_RATE).coerceIn(SNAPSHOT_RATES)
        set(value) = node.putInt(SNAPSHOT_RATE, value.coerceIn(SNAPSHOT_RATES))

    override var skipStopConfirmation: Boolean
        get() = node.getBoolean(SKIP_STOP, false)
        set(value) = node.putBoolean(SKIP_STOP, value)

    override var skipPauseAllConfirmation: Boolean
        get() = node.getBoolean(SKIP_PAUSE_ALL, false)
        set(value) = node.putBoolean(SKIP_PAUSE_ALL, value)

    override fun get(serverId: String): Long? = node.get(seqKey(serverId), null)?.toLongOrNull()

    override fun set(serverId: String, seq: Long) = node.putLong(seqKey(serverId), seq)

    /** A server id too long for a preferences key is cut and told apart by its hash. */
    private fun seqKey(serverId: String): String {
        val key = LAST_SEQ + serverId
        if (key.length <= Preferences.MAX_KEY_LENGTH) return key
        val hash = "#" + Integer.toHexString(serverId.hashCode())
        return key.take(Preferences.MAX_KEY_LENGTH - hash.length) + hash
    }

    private companion object {
        const val NOTIFY_FINISHED = "notify.finished"
        const val NOTIFY_FAILED = "notify.failed"
        const val KEEP_IN_TRAY = "keepRunningInTray"
        const val SNAPSHOT_RATE = "snapshotRate"
        const val SKIP_STOP = "skipConfirmation.stop"
        const val SKIP_PAUSE_ALL = "skipConfirmation.pauseAll"
        const val LAST_SEQ = "lastSeq."
    }
}

/** [AppPreferences] that last as long as the object, for tests. */
class InMemoryAppPreferences : AppPreferences {
    @Volatile override var notifyFinished = true

    @Volatile override var notifyFailed = true

    @Volatile override var keepRunningInTray = true

    @Volatile override var snapshotRate = DEFAULT_SNAPSHOT_RATE
        set(value) {
            field = value.coerceIn(SNAPSHOT_RATES)
        }

    @Volatile override var skipStopConfirmation = false

    @Volatile override var skipPauseAllConfirmation = false

    private val seqs = ConcurrentHashMap<String, Long>()

    override fun get(serverId: String): Long? = seqs[serverId]

    override fun set(serverId: String, seq: Long) {
        seqs[serverId] = seq
    }
}
