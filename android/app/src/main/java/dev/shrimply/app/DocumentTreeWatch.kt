package dev.shrimply.app

import android.content.ContentResolver
import android.database.ContentObserver
import android.database.Cursor
import android.net.Uri
import android.os.Bundle
import android.provider.DocumentsContract
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.withTimeoutOrNull
import java.io.File
import java.util.concurrent.TimeUnit

/** A SAF directory listing's cursor owns its provider-side directory watch. */
internal class DocumentTreeWatch(
    private val resolver: ContentResolver,
    val tree: Uri,
) : AutoCloseable {
    @Volatile var project: File? = null
    private val updates = Channel<Unit>(Channel.CONFLATED)
    private val cursors = mutableMapOf<String, Cursor>()
    private val incoming = mutableSetOf<String>()
    private var checkUnversioned = true
    private var revision = 0L
    private var closed = false
    @Volatile private var poll = false

    // Each live provider cursor holds Binder and file-observer resources. For large
    // trees and providers without notifications, use bounded periodic reconciliation.
    private companion object {
        const val MAX_CURSORS = 64
        val POLL_MS = TimeUnit.SECONDS.toMillis(30)
    }
    init { updates.trySend(Unit) }

    data class Changes(val directories: Set<String>, val checkUnversioned: Boolean, val revision: Long)

    @Synchronized fun changes() = Changes(incoming.toSet(), checkUnversioned, revision)

    @Synchronized fun acknowledge(changes: Changes) {
        if (revision == changes.revision) {
            incoming.clear()
            checkUnversioned = false
        }
    }

    @Synchronized private fun remoteChanged(id: String) {
        if (closed) return
        revision++
        if ("" in incoming) { updates.trySend(Unit); return }
        if (id.isEmpty() || incoming.size >= MAX_CURSORS) { incoming.clear(); incoming.add("") } else incoming.add(id)
        updates.trySend(Unit)
    }

    fun localChanged() { updates.trySend(Unit) }

    @Synchronized fun resumed() {
        revision++
        checkUnversioned = true
        updates.trySend(Unit)
    }

    suspend fun awaitChange(): Boolean {
        if (!poll) return updates.receiveCatching().isSuccess
        // A provider without notifications may still report size or last-modified
        // changes. Recheck metadata here; re-read unversioned files on resume.
        return withTimeoutOrNull(POLL_MS) { updates.receiveCatching().isSuccess } ?: true
    }

    // Only the synchronization worker creates subscriptions. Release the lock during
    // provider IPC so detaching a slow cloud folder cannot stall the Android UI.
    fun observe(id: String) {
        synchronized(this) {
            if (closed || id in cursors) return
            if (cursors.size >= MAX_CURSORS) { poll = true; return }
        }
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, id)
        val query = Bundle().apply { putInt(ContentResolver.QUERY_ARG_LIMIT, 1) }
        val cursor = resolver.query(children, arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID), query, null)
        if (cursor == null) { poll = true; return }
        try {
            cursor.registerContentObserver(object : ContentObserver(null) {
                override fun onChange(selfChange: Boolean) = remoteChanged(id)
            })
            if (cursor.notificationUris.isNullOrEmpty()) poll = true
            synchronized(this) {
                if (!closed && id !in cursors) {
                    cursors[id] = cursor
                    return
                }
            }
        } catch (e: Exception) {
            cursor.close()
            throw e
        }
        cursor.close()
    }

    fun retain(ids: Set<String>) {
        val removed = synchronized(this) { (cursors.keys - ids).mapNotNull(cursors::remove) }
        removed.forEach(Cursor::close)
    }

    @Synchronized override fun close() {
        closed = true
        updates.close()
        // The worker releases the provider cursors in its finally block off the UI thread.
    }
}
