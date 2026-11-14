package dev.shrimply.app

import android.content.Context
import android.database.ContentObserver
import android.net.Uri
import android.os.FileObserver
import android.os.Handler
import android.os.Looper
import android.provider.DocumentsContract
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.io.File
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.security.MessageDigest
import java.util.UUID

/** Android document providers are mirrored to real paths for the shared project backend.
 * Files retain their relative paths; content is streamed, never loaded as one large byte array.
 * The persisted tree grant and origin survive process death. Native watches observe this mirror.
 */
internal class ProjectFiles(context: Context) : AutoCloseable {
    private val context = context.applicationContext
    private companion object {
        const val MAX_MIRRORED_ENTRIES = 16_384
        val PRIVATE_FILE = Regex("(?:.*\\.(?:shrimp|sjson|json)\\.lock|.*\\.(?:shrimp|sjson|json)\\.tmp-\\d+(?:-\\d+)?|\\.incoming-[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})")
    }
    private val resolver = context.contentResolver
    private val origins = context.getSharedPreferences("project-origins", Context.MODE_PRIVATE)
    private val root = File(context.filesDir, "projects").apply { mkdirs() }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val mutex = Mutex()
    private val main = Handler(Looper.getMainLooper())
    private var contentObserver: ContentObserver? = null
    private val observers = mutableMapOf<String, FileObserver>()
    private val watchLock = Any()
    private var sync: Channel<Unit>? = null
    private var sourceTree: Uri? = null
    private val sourceFiles = mutableMapOf<String, Pair<Entry, File>>()
    private var onError: (String) -> Unit = {}
    @Volatile private var generation = 0
    @Volatile private var closing = false
    @Volatile private var closed = false
    private val mirrorDatabase = lazy {
        this.context.openOrCreateDatabase("project-mirrors.sqlite", Context.MODE_PRIVATE, null).apply {
            execSQL("CREATE TABLE IF NOT EXISTS fingerprints (uri TEXT PRIMARY KEY, hash TEXT NOT NULL)")
            execSQL("CREATE TABLE IF NOT EXISTS source_entries (uri TEXT PRIMARY KEY, tree TEXT NOT NULL, path TEXT NOT NULL, mime TEXT NOT NULL)")
        }
    }
    private fun baseline(uri: Uri): String? = mirrorDatabase.value.rawQuery(
        "SELECT hash FROM fingerprints WHERE uri=?", arrayOf(uri.toString())
    ).use { if (it.moveToFirst()) it.getString(0) else null }
    private fun rememberHash(uri: Uri, hash: ByteArray) {
        mirrorDatabase.value.execSQL("INSERT INTO fingerprints(uri,hash) VALUES (?,?) ON CONFLICT(uri) DO UPDATE SET hash=excluded.hash WHERE hash!=excluded.hash",
            arrayOf(uri.toString(), hash.joinToString("") { "%02x".format(it) }))
    }
    private fun hashLabel(hash: ByteArray?) = hash?.joinToString("") { "%02x".format(it) }
    private fun ensureOpen() { if (closed) throw CancellationException("Project session closed") }

    fun newPath(name: String): File {
        check(!closing) { "Project session closed" }
        val folder = File(root, UUID.randomUUID().toString())
        val fileName = name.trim().replace(Regex("[\\\\/:*?\"<>|]"), "_").ifEmpty { "Untitled" }
        return File(folder, "$fileName.shrimp")
    }

    suspend fun openTree(tree: Uri, progress: (String) -> Unit): List<File> = withContext(Dispatchers.IO) {
        mutex.withLock {
            check(!closing) { "Project session closed" }
            val existing = origins.all.entries.firstOrNull { it.value == tree.toString() }?.key?.let(::File)
            val folder = existing?.takeIf(File::isDirectory) ?: File(root, UUID.randomUUID().toString()).apply { check(mkdir()) }
            try {
                if (sourceTree != tree) restoreSources(tree, folder)
                copyFrom(tree, DocumentsContract.getTreeDocumentId(tree), folder, progress)
                val projects = folder.walkTopDown().onEnter { it.name != ".snapshots" }.filter { it.isFile && it.extension.equals("shrimp", true) }.toList()
                check(projects.isNotEmpty()) { "This folder does not contain a .shrimp project." }
                check(origins.edit().putString(folder.absolutePath, tree.toString()).commit()) { "Could not remember the project folder." }
                projects
            } catch (error: Exception) {
                if (existing == null) folder.deleteRecursively()
                throw error
            }
        }
    }

    suspend fun exportTree(projectPath: String, tree: Uri, progress: (String) -> Unit) = withContext(Dispatchers.IO) {
        mutex.withLock {
            check(!closing) { "Project session closed" }
            val folder = projectRoot(File(projectPath))
            copyTo(folder, tree, DocumentsContract.getTreeDocumentId(tree), progress = progress)
        }
    }

    suspend fun exportCompletedFile(path: String, destination: Uri) = withContext(Dispatchers.IO) {
        mutex.withLock {
            check(!closing) { "Project session closed" }
            val source = File(path).canonicalFile
            check(source.toPath().startsWith(File(context.cacheDir, "exports").canonicalFile.toPath()) && source.isFile) { "The completed export is unavailable." }
            source.inputStream().use { input ->
                checkNotNull(resolver.openOutputStream(destination, "wt")) { "Could not open the export destination." }.use { output ->
                    check(input.copyTo(output) == source.length()) { "The export changed while it was being saved." }
                    output.flush()
                }
            }
        }
    }

    suspend fun exportCompletedFolder(path: String, destination: Uri, mime: String, progress: (String) -> Unit) = withContext(Dispatchers.IO) {
        mutex.withLock {
            check(!closing) { "Project session closed" }
            val folder = File(path).canonicalFile
            check(folder.toPath().startsWith(File(context.cacheDir, "exports").canonicalFile.toPath()) && folder.isDirectory) { "The completed export is unavailable." }
            copyTo(folder, destination, DocumentsContract.getTreeDocumentId(destination), mime, progress)
        }
    }

    suspend fun importFile(source: Uri, consume: suspend (String, String, java.io.FileDescriptor) -> Unit) = withContext(Dispatchers.IO) {
        // Directory CREATE events can trigger recursive mirror scans. Keep those
        // scans behind the complete native import, including validation/rollback.
        mutex.withLock {
            check(!closing) { "Project session closed" }
            val name = resolver.query(source, arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME), null, null, null)?.use {
                if (it.moveToFirst()) it.getString(0) else null
            } ?: throw IllegalArgumentException("The selected file has no name.")
            checkName(name)
            resolver.openFileDescriptor(source, "r")?.use { descriptor ->
                // Keep the provider descriptor alive until the native transaction completes.
                // Native import reads a bounded source once and owns the durable asset copy.
                // Coroutine cancellation cannot release a descriptor still queued on the native
                // project worker. Wait for that transaction's response, then release the lease.
                withContext(NonCancellable) { consume(name, "/proc/self/fd/${descriptor.fd}", descriptor.fileDescriptor) }
            } ?: throw IllegalArgumentException("Could not open the selected file.")
        }
    }

    suspend fun flush(projectPath: String) = withContext(Dispatchers.IO) {
        mutex.withLock {
            check(!closing) { "Project session closed" }
            val folder = projectRoot(File(projectPath))
            origins.getString(folder.absolutePath, null)?.let(Uri::parse)?.let { tree ->
                copyTo(folder, tree, DocumentsContract.getTreeDocumentId(tree)) {}
            }
        }
    }

    fun attach(projectPath: String?, error: (String) -> Unit): Int {
        detach()
        check(!closing) { "Project session closed" }
        onError = error
        if (projectPath == null) return generation
        val folder = projectRoot(File(projectPath))
        val tree = origins.getString(folder.absolutePath, null)?.let(Uri::parse) ?: return generation
        val epoch = generation
        val incoming = mutableSetOf<String>()
        val outgoing = mutableSetOf<String>()
        val updates = Channel<Unit>(Channel.CONFLATED)
        sync = updates
        scope.launch {
            try {
                mutex.withLock { if (epoch == generation && sourceTree != tree) restoreSources(tree, folder) }
                watchLocal(folder, epoch) { changed -> queueChange(outgoing, changed, "", updates) }
            } catch (failure: Exception) { report(epoch, failure); return@launch }
            for (ignored in updates) {
                try {
                    mutex.withLock {
                        if (epoch != generation) return@launch
                        if (sourceTree != tree) restoreSources(tree, folder)
                        val changed = takeChanges(outgoing)
                        // Apply known local deletions before a provider echo can restore them.
                        if (deleteMissingSources(tree, folder, changed)) {
                            watchLocal(folder, epoch) { next -> queueChange(outgoing, next, "", updates) }
                        }
                        val remoteChanges = takeChanges(incoming)
                        if (remoteChanges.isNotEmpty()) {
                            var fullScan = false
                            for (changed in remoteChanges) {
                                val id = runCatching { DocumentsContract.getDocumentId(Uri.parse(changed)) }.getOrNull()
                                val known = sourceFiles[id]
                                if (known == null || known.second.isDirectory) fullScan = true
                                else syncFile(tree, known.first, known.second) {}
                            }
                            if (fullScan) {
                                copyFrom(tree, DocumentsContract.getTreeDocumentId(tree), folder) {}
                                watchLocal(folder, epoch) { changed -> queueChange(outgoing, changed, "", updates) }
                            }
                        }
                        for (relative in changed) {
                            val file = File(folder, relative)
                            if (file.isFile) writeChangedFile(folder, file, tree)
                            else if (file.isDirectory) {
                                writeChangedDirectory(folder, file, tree)
                                watchLocal(folder, epoch) { next -> queueChange(outgoing, next, "", updates) }
                            }
                        }
                    }
                } catch (failure: Exception) { report(epoch, failure) }
            }
        }
        val observer = object : ContentObserver(main) {
            override fun onChange(selfChange: Boolean) { queueChange(incoming, tree.toString(), tree.toString(), updates) }
            override fun onChange(selfChange: Boolean, uri: Uri?) { queueChange(incoming, (uri ?: tree).toString(), tree.toString(), updates) }
        }
        try {
            resolver.registerContentObserver(tree, true, observer)
            contentObserver = observer
        } catch (failure: Exception) { report(epoch, failure) }
        return epoch
    }

    private fun queueChange(changes: MutableSet<String>, path: String, full: String, updates: Channel<Unit>) {
        synchronized(changes) {
            if (full !in changes) {
                if (changes.size >= MAX_MIRRORED_ENTRIES) { changes.clear(); changes.add(full) }
                else changes.add(path)
            }
        }
        updates.trySend(Unit)
    }
    private fun takeChanges(changes: MutableSet<String>): List<String> = synchronized(changes) {
        changes.toList().also { changes.clear() }
    }

    private fun watchLocal(folder: File, epoch: Int, changed: (String) -> Unit) {
        synchronized(watchLock) {
            val removed = observers.keys.filter { !File(it).isDirectory }
            removed.forEach { observers.remove(it)?.stopWatching() }
        }
        folder.walkTopDown().filter(File::isDirectory).forEach { directory ->
            synchronized(watchLock) {
                if (epoch != generation || observers.containsKey(directory.absolutePath)) return@forEach
                check(observers.size < MAX_MIRRORED_ENTRIES) { "The project folder exceeds its directory budget." }
                observers[directory.absolutePath] = object : FileObserver(directory, CLOSE_WRITE or MOVED_TO or CREATE or DELETE or MOVED_FROM) {
                    override fun onEvent(event: Int, path: String?) {
                        if (path != null && !ignored(path)) {
                            val file = File(directory, path)
                            // A created regular file can still be copying. Publish on close/rename.
                            if (event and (CLOSE_WRITE or MOVED_TO or DELETE or MOVED_FROM) != 0 || file.isDirectory) changed(file.relativeTo(folder).path)
                        }
                    }
                }.also(FileObserver::startWatching)
            }
        }
    }

    private fun destinationDirectory(root: File, directory: File, tree: Uri): String {
        var parent = DocumentsContract.getTreeDocumentId(tree)
        var local = root
        for (part in directory.relativeTo(root).path.split(File.separatorChar).filter(String::isNotEmpty)) {
            val existing = entries(tree, parent).firstOrNull { it.name == part }
            check(existing == null || existing.mime == DocumentsContract.Document.MIME_TYPE_DIR) { "'$part' is a file in the destination folder." }
            parent = existing?.id ?: DocumentsContract.createDocument(resolver,
                DocumentsContract.buildDocumentUriUsingTree(tree, parent), DocumentsContract.Document.MIME_TYPE_DIR, part)
                ?.let(DocumentsContract::getDocumentId) ?: throw IllegalArgumentException("Could not create '$part'.")
            local = File(local, part)
            if (tree == sourceTree) rememberEntry(tree, Entry(parent, part, DocumentsContract.Document.MIME_TYPE_DIR), local)
        }
        return parent
    }
    private fun writeChangedFile(root: File, file: File, tree: Uri) {
        val parent = destinationDirectory(root, file.parentFile!!, tree)
        writeFile(file, tree, parent, entries(tree, parent).firstOrNull { it.name == file.name })
    }
    private fun writeChangedDirectory(root: File, directory: File, tree: Uri) {
        copyTo(directory, tree, destinationDirectory(root, directory, tree)) {}
    }

    private fun report(epoch: Int, failure: Exception) {
        main.post { if (epoch == generation) onError("Could not synchronize project: ${failure.message}") }
    }

    private fun projectRoot(file: File): File {
        val canonicalRoot = root.canonicalFile
        return generateSequence(file.canonicalFile.parentFile) { it.parentFile }
            .firstOrNull { it.parentFile == canonicalRoot }
            ?: throw IllegalArgumentException("Project is outside Android's managed project directory.")
    }

    private data class Entry(val id: String, val name: String, val mime: String)
    private fun rememberEntry(tree: Uri, entry: Entry, file: File) {
        check(sourceFiles.containsKey(entry.id) || sourceFiles.size < MAX_MIRRORED_ENTRIES) { "The project folder exceeds its file budget." }
        sourceFiles[entry.id] = entry to file
        mirrorDatabase.value.execSQL("INSERT INTO source_entries(uri,tree,path,mime) VALUES (?,?,?,?) ON CONFLICT(uri) DO UPDATE SET tree=excluded.tree,path=excluded.path,mime=excluded.mime WHERE tree!=excluded.tree OR path!=excluded.path OR mime!=excluded.mime",
            arrayOf(DocumentsContract.buildDocumentUriUsingTree(tree, entry.id).toString(), tree.toString(), file.absolutePath, entry.mime))
    }
    private fun restoreSources(tree: Uri, folder: File) {
        sourceFiles.clear()
        sourceTree = tree
        mirrorDatabase.value.rawQuery("SELECT uri,path,mime FROM source_entries WHERE tree=?", arrayOf(tree.toString())).use { cursor ->
            while (cursor.moveToNext()) {
                val file = File(cursor.getString(1))
                if (!file.toPath().startsWith(folder.toPath())) continue
                check(sourceFiles.size < MAX_MIRRORED_ENTRIES) { "The project folder exceeds its file budget." }
                val id = DocumentsContract.getDocumentId(Uri.parse(cursor.getString(0)))
                sourceFiles[id] = Entry(id, file.name, cursor.getString(2)) to file
            }
        }
    }

    private fun entries(tree: Uri, parent: String): List<Entry> {
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, parent)
        return resolver.query(children, arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE), null, null, null)?.use { cursor ->
            val names = mutableSetOf<String>()
            buildList { while (cursor.moveToNext()) {
                check(size < MAX_MIRRORED_ENTRIES) { "The provider folder exceeds its file budget." }
                val entry = Entry(cursor.getString(0), cursor.getString(1), cursor.getString(2))
                checkName(entry.name)
                check(names.add(entry.name)) { "The provider returned duplicate file names." }
                add(entry)
            } }
        } ?: throw IllegalArgumentException("The document provider could not list the project folder.")
    }

    private fun inspect(tree: Uri, id: String): Entry? {
        val uri = DocumentsContract.buildDocumentUriUsingTree(tree, id)
        val cursor = resolver.query(uri, arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE), null, null, null)
            ?: throw IllegalArgumentException("Could not inspect the source document.")
        return cursor.use {
            if (!it.moveToFirst()) null else Entry(it.getString(0), it.getString(1), it.getString(2)).also { entry -> checkName(entry.name) }
        }
    }

    private fun checkName(name: String) {
        require(name.isNotBlank() && name != "." && name != ".." && !name.contains('/') && !name.contains('\\')) {
            "The document provider returned an invalid file name."
        }
    }
    private fun ignored(name: String) = PRIVATE_FILE.matches(name)

    private fun copyFrom(tree: Uri, parent: String, folder: File, progress: (String) -> Unit) {
        check(folder.isDirectory || folder.mkdirs()) { "Could not create the project folder." }
        val children = entries(tree, parent)
        val identifiers = children.map(Entry::id).toSet()
        val childrenByName = children.associateBy(Entry::name)
        val removed = sourceFiles.filter { (id, value) -> value.second.parentFile == folder && id !in identifiers }
        for ((id, value) in removed) {
            val replacement = childrenByName[value.first.name]
            if (replacement == null) removeDeletedSource(tree, id, value.second)
            else {
                check(replacement.mime == value.first.mime) { "'${replacement.name}' changed file type externally. The local version was kept." }
                // Providers often replace a document ID when saving atomically. Transfer
                // its baseline before syncFile atomically replaces the local inode.
                val oldUri = DocumentsContract.buildDocumentUriUsingTree(tree, id).toString()
                val newUri = DocumentsContract.buildDocumentUriUsingTree(tree, replacement.id).toString()
                mirrorDatabase.value.beginTransaction()
                try {
                    mirrorDatabase.value.execSQL("INSERT INTO fingerprints(uri,hash) SELECT ?,hash FROM fingerprints WHERE uri=? ON CONFLICT(uri) DO UPDATE SET hash=excluded.hash", arrayOf(newUri, oldUri))
                    mirrorDatabase.value.execSQL("DELETE FROM fingerprints WHERE uri=?", arrayOf(oldUri))
                    mirrorDatabase.value.execSQL("DELETE FROM source_entries WHERE uri=?", arrayOf(oldUri))
                    rememberEntry(tree, replacement, value.second)
                    mirrorDatabase.value.setTransactionSuccessful()
                } finally { mirrorDatabase.value.endTransaction() }
                sourceFiles.remove(id)
            }
        }
        for (entry in children) {
            ensureOpen()
            if (ignored(entry.name)) continue
            val target = File(folder, entry.name)
            sourceFiles[entry.id]?.let { (previous, file) ->
                check(file == target && previous.mime == entry.mime) { "'${previous.name}' was renamed or changed file type externally. The local version was kept; export local changes before reopening the source." }
            }
            check(sourceFiles.containsKey(entry.id) || sourceFiles.size < MAX_MIRRORED_ENTRIES) { "The selected folder contains too many files. Choose the folder containing this project and its media." }
            rememberEntry(tree, entry, target)
            if (entry.mime == DocumentsContract.Document.MIME_TYPE_DIR) copyFrom(tree, entry.id, target, progress)
            else syncFile(tree, entry, target, progress)
        }
    }

    private fun removeDeletedSource(tree: Uri, id: String, target: File) {
        // A remote deletion must not erase newer or newly created local descendants.
        val known = if (target.isDirectory) sourceFiles.entries.associate { it.value.second.absolutePath to it.key } else emptyMap()
        for (file in target.walkTopDown().filter { it.isFile && !ignored(it.name) }) {
            val sourceId = if (file == target) id else known[file.absolutePath]
            val base = sourceId?.let { baseline(DocumentsContract.buildDocumentUriUsingTree(tree, it)) }
            check(base != null && hashLabel(file.inputStream().use { digest(it) }) == base) {
                "'${file.name}' was deleted externally while this device has newer or unsynchronized edits. Export the local project before reopening its source."
            }
        }
        check(!target.exists() || target.deleteRecursively()) { "Could not remove '${target.name}' after its source was deleted." }
        forgetSources(tree, listOf(target))
    }

    private fun forgetSources(tree: Uri, targets: List<File>) {
        val paths = targets.toSet()
        val removed = sourceFiles.filter { (_, value) -> generateSequence(value.second) { it.parentFile }.any { it in paths } }.keys
        mirrorDatabase.value.beginTransaction()
        try {
            for (sourceId in removed) {
                val source = arrayOf(DocumentsContract.buildDocumentUriUsingTree(tree, sourceId).toString())
                mirrorDatabase.value.execSQL("DELETE FROM fingerprints WHERE uri=?", source)
                mirrorDatabase.value.execSQL("DELETE FROM source_entries WHERE uri=?", source)
            }
            mirrorDatabase.value.setTransactionSuccessful()
        } finally { mirrorDatabase.value.endTransaction() }
        sourceFiles.keys.removeAll(removed.toSet())
    }

    private fun deleteMissingSources(tree: Uri, folder: File, changed: List<String>): Boolean {
        if (changed.isEmpty()) return false
        val missing = changed.map { File(folder, it) }.filter { !it.exists() }.toSet()
        val full = "" in changed
        if (!full && missing.isEmpty()) return false
        val affected = sourceFiles.filter { (_, value) ->
            val file = value.second
            !file.exists() && (full || generateSequence(file) { it.parentFile?.takeIf { parent -> parent.toPath().startsWith(folder.toPath()) } }.any { it in missing })
        }
        val paths = affected.values.map { it.second }.toSet()
        val roots = affected.values.filter { (_, file) -> generateSequence(file.parentFile) { it.parentFile }.none { it in paths } }
        val directories = sourceFiles.values.filter { it.first.mime == DocumentsContract.Document.MIME_TYPE_DIR }.associate { it.second to it.first }
        val parentListings = mutableMapOf<String, Map<String, Entry>>()
        // Preflight every descendant before deleting a provider directory. Unknown,
        // renamed or externally modified children preserve the entire remote subtree.
        for (root in roots) {
            val pending = ArrayDeque<Pair<Entry, File>>()
            pending.add(root)
            while (pending.isNotEmpty()) {
                val (expected, file) = pending.removeFirst()
                val current = inspect(tree, expected.id) ?: continue
                check(current == expected) { "'${expected.name}' changed externally while being removed locally. The source was kept." }
                if (file == root.second) {
                    var ancestor = file
                    var identity = expected
                    while (ancestor != folder) {
                        val parent = ancestor.parentFile ?: error("The mirrored path has no parent.")
                        val parentId = if (parent == folder) DocumentsContract.getTreeDocumentId(tree)
                            else directories[parent]?.id ?: error("The source parent is not tracked. Its contents were kept.")
                        val listing = parentListings.getOrPut(parentId) {
                            entries(tree, parentId).filter { it.id in sourceFiles }.associateBy(Entry::id)
                        }
                        check(listing[identity.id] == identity) { "'${expected.name}' or its source folder moved externally. The source was kept." }
                        if (parent == folder) break
                        identity = directories.getValue(parent)
                        ancestor = parent
                    }
                }
                if (current.mime == DocumentsContract.Document.MIME_TYPE_DIR) {
                    for (child in entries(tree, current.id)) {
                        val known = affected[child.id]
                        check(known != null && known.first == child && known.second == File(file, child.name)) {
                            "'${expected.name}' contains new or unsynchronized source files. The source folder was kept."
                        }
                        pending.add(known)
                    }
                } else {
                    val uri = DocumentsContract.buildDocumentUriUsingTree(tree, current.id)
                    val remote = resolver.openInputStream(uri)?.use { hashLabel(digest(it)) }
                        ?: throw IllegalArgumentException("Could not read '${current.name}' before deleting it.")
                    check(baseline(uri)?.let { it == remote } == true) {
                        "'${current.name}' changed externally while being removed locally. The source was kept."
                    }
                }
            }
        }
        val deleted = mutableListOf<File>()
        try {
            for ((entry, target) in roots) {
                check(!target.exists()) { "'${entry.name}' was recreated locally during synchronization. The source was kept." }
                val current = inspect(tree, entry.id)
                check(current == null || current == entry) { "'${entry.name}' changed externally during deletion. The source was kept." }
                if (current != null) {
                    check(DocumentsContract.deleteDocument(resolver, DocumentsContract.buildDocumentUriUsingTree(tree, entry.id))) { "Could not delete source '${entry.name}'." }
                }
                deleted.add(target)
            }
        } finally {
            // One metadata scan/transaction for the successful subset, including a
            // partially failed batch. Do not forget provider deletions that failed.
            if (deleted.isNotEmpty()) forgetSources(tree, deleted)
        }
        return affected.values.any { it.first.mime == DocumentsContract.Document.MIME_TYPE_DIR }
    }

    private fun syncFile(tree: Uri, entry: Entry, target: File, progress: (String) -> Unit) {
        progress(entry.name)
        val uri = DocumentsContract.buildDocumentUriUsingTree(tree, entry.id)
        try {
            val current = inspect(tree, entry.id) ?: throw java.io.FileNotFoundException(entry.name)
            check(current == entry) { "'${entry.name}' was renamed or changed file type externally. The local version was kept." }
            val remoteHash = resolver.openInputStream(uri)?.use { digest(it) }
                ?: throw IllegalArgumentException("Could not read '${entry.name}'.")
            val localHash = target.takeIf(File::isFile)?.inputStream()?.use { digest(it) }
            if (remoteHash.contentEquals(localHash)) { rememberHash(uri, remoteHash); return }
            val base = baseline(uri)
            if (localHash != null && hashLabel(localHash) != base) {
                check(base != null && hashLabel(remoteHash) == base) { "'${entry.name}' has different changes on this device and in its source folder. Export the local project before reopening its source." }
                writeChangedFile(projectRoot(target), target, tree)
                return
            }
            val incoming = File(target.parentFile, ".incoming-${UUID.randomUUID()}")
            try {
                resolver.openInputStream(uri)?.use { input -> incoming.outputStream().use { output -> copy(input, output) } }
                    ?: throw IllegalArgumentException("Could not read '${entry.name}'.")
                check(incoming.inputStream().use { digest(it) }.contentEquals(remoteHash)) {
                    "'${entry.name}' changed while being read. The local version was kept; retry synchronization."
                }
                check(target.takeIf(File::isFile)?.inputStream()?.use { digest(it) }.let { latest ->
                    if (localHash == null) latest == null else latest != null && latest.contentEquals(localHash)
                }) { "'${entry.name}' changed locally while synchronizing. The local version was kept; retry synchronization." }
                Files.move(incoming.toPath(), target.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
                rememberHash(uri, remoteHash)
            } finally { incoming.delete() }
        } catch (missing: java.io.FileNotFoundException) {
            val deleted = resolver.query(uri, arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID), null, null, null)?.use { it.count == 0 } == true
            if (!deleted) throw missing
            removeDeletedSource(tree, entry.id, target)
        }
    }

    private fun copyTo(folder: File, tree: Uri, parent: String, mime: String = "application/octet-stream", progress: (String) -> Unit) {
        val destination = entries(tree, parent).associateBy(Entry::name)
        // Publish media first and the project document last.
        for (file in folder.listFiles()?.sortedBy { it.extension == "shrimp" }
            ?: throw IllegalArgumentException("Could not read the project folder.")) {
            ensureOpen()
            if (ignored(file.name)) continue
            progress(file.name)
            val existing = destination[file.name]
            if (file.isDirectory) {
                check(existing == null || existing.mime == DocumentsContract.Document.MIME_TYPE_DIR) { "'${file.name}' is a file in the destination folder." }
                val uri = existing?.let { DocumentsContract.buildDocumentUriUsingTree(tree, it.id) }
                    ?: DocumentsContract.createDocument(resolver, DocumentsContract.buildDocumentUriUsingTree(tree, parent), DocumentsContract.Document.MIME_TYPE_DIR, file.name)
                    ?: throw IllegalArgumentException("Could not create '${file.name}'.")
                val id = DocumentsContract.getDocumentId(uri)
                if (tree == sourceTree) rememberEntry(tree, Entry(id, file.name, DocumentsContract.Document.MIME_TYPE_DIR), file)
                copyTo(file, tree, id, mime, progress)
            } else {
                writeFile(file, tree, parent, existing, mime)
            }
        }
    }

    private fun writeFile(file: File, tree: Uri, parent: String, existing: Entry?, mime: String = "application/octet-stream") {
        check(existing?.mime != DocumentsContract.Document.MIME_TYPE_DIR) { "'${file.name}' is a directory in the destination folder." }
        // Lease the local inode before creating a remote document. A failed native
        // import may already have removed the path by the time its event is handled.
        file.inputStream().use { input ->
        val localHash = digest(input)
        input.channel.position(0)
        check(tree != sourceTree || existing != null || sourceFiles.size < MAX_MIRRORED_ENTRIES) { "The project folder exceeds its file budget." }
        val uri = existing?.let { DocumentsContract.buildDocumentUriUsingTree(tree, it.id) }
            ?: DocumentsContract.createDocument(resolver, DocumentsContract.buildDocumentUriUsingTree(tree, parent), mime, file.name)
            ?: throw IllegalArgumentException("Could not create '${file.name}'.")
        val entry = Entry(DocumentsContract.getDocumentId(uri), file.name, existing?.mime ?: mime)
        if (tree == sourceTree) rememberEntry(tree, entry, file)
        if (existing != null) {
            val remoteHash = resolver.openInputStream(uri)?.use { digest(it) }
                ?: throw IllegalArgumentException("Could not read destination '${file.name}'.")
            if (remoteHash.contentEquals(localHash)) { rememberHash(uri, localHash); return }
            val base = baseline(uri)
            check(base != null && (hashLabel(remoteHash) == base || hashLabel(localHash) == base)) {
                "'${file.name}' already has different content in the destination folder. Export to an empty folder to preserve both versions."
            }
            if (hashLabel(localHash) == base) {
                check(tree == sourceTree) { "The export destination has newer content. Choose an empty folder." }
                syncFile(tree, entry, file) {}
                return
            }
        }
        val copied = MessageDigest.getInstance("SHA-256")
        resolver.openOutputStream(uri, "rwt")?.use { output -> copy(java.security.DigestInputStream(input, copied), output) }
            ?: throw IllegalArgumentException("Could not write '${file.name}'.")
        check(copied.digest().contentEquals(localHash)) { "'${file.name}' changed during upload. The local version was kept; retry synchronization." }
        rememberHash(uri, localHash)
        }
    }

    private fun copy(input: java.io.InputStream, output: java.io.OutputStream) {
        val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
        while (true) {
            ensureOpen()
            val length = input.read(buffer)
            if (length < 0) return
            output.write(buffer, 0, length)
        }
    }

    private fun digest(input: java.io.InputStream): ByteArray {
        val hash = MessageDigest.getInstance("SHA-256")
        val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
        while (true) {
            ensureOpen()
            val length = input.read(buffer)
            if (length < 0) break
            hash.update(buffer, 0, length)
        }
        return hash.digest()
    }

    fun detach(owner: Int? = null) {
        if (owner != null && owner != generation) return
        generation++
        contentObserver?.let(resolver::unregisterContentObserver)
        contentObserver = null
        sync?.close()
        sync = null
        synchronized(watchLock) {
            observers.values.forEach(FileObserver::stopWatching)
            observers.clear()
        }
    }
    override fun close() {
        if (closing) return
        closing = true
        detach()
        onError = {}
        scope.launch {
            // An output descriptor may already have truncated a provider document.
            // Finish its active transfer before shutdown can make copy() abort.
            mutex.withLock {
                closed = true
                if (mirrorDatabase.isInitialized()) mirrorDatabase.value.close()
            }
            scope.cancel()
        }
    }
}
