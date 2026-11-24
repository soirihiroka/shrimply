package dev.shrimply.app

import android.content.Context
import android.content.ActivityNotFoundException
import android.content.Intent
import android.database.ContentObserver
import android.net.Uri
import androidx.core.database.sqlite.transaction
import org.json.JSONObject
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import android.os.Handler
import android.os.Looper
import android.provider.DocumentsContract
import kotlinx.coroutines.*
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
        val mutex = Mutex()
        const val MAX_MIRRORED_ENTRIES = 16_384
        // FAT-backed providers can round modification times down to two seconds.
        val PROVIDER_TIME_RESOLUTION_MS = TimeUnit.SECONDS.toMillis(2)
        const val UPLOAD_CLEANUP = 0
        const val UPLOAD_STAGED = 1
        const val UPLOAD_WRITING = 2
        const val UPLOAD_CREATING = 3
        const val UPLOAD_RESTORING = 4
        val PRIVATE_FILE = Regex("(?:.*\\.(?:shrimp|sjson|json)\\.lock|.*\\.(?:shrimp|sjson|json)\\.tmp-\\d+(?:-\\d+)?|\\.incoming-[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})")
    }
    private val resolver = context.contentResolver
    private val origins = context.getSharedPreferences("project-origins", Context.MODE_PRIVATE)
    private val root = File(context.filesDir, "projects").apply { mkdirs() }
    private val jobs = SupervisorJob()
    private val scope = CoroutineScope(jobs + Dispatchers.IO)
    private val main = Handler(Looper.getMainLooper())
    @Volatile private var watch: DocumentTreeWatch? = null
    private var pendingWatch: DocumentTreeWatch? = null
    @Volatile private var localChange: ((String, String) -> Unit)? = null
    private var sourceTree: Uri? = null
    private val sourceFiles = mutableMapOf<String, Pair<Entry, File>>()
    private var onError: (String) -> Unit = {}
    @Volatile private var generation = 0
    @Volatile private var closing = false
    @Volatile private var closed = false
    private val mirrorDatabase = lazy {
        this.context.openOrCreateDatabase("project-mirrors.sqlite", Context.MODE_PRIVATE, null).apply {
            execSQL("CREATE TABLE IF NOT EXISTS relocations (tree TEXT PRIMARY KEY, document TEXT NOT NULL, source TEXT NOT NULL, target TEXT NOT NULL, inode TEXT NOT NULL)")
            execSQL("CREATE TABLE IF NOT EXISTS uploads (stage TEXT PRIMARY KEY, tree TEXT NOT NULL, parent TEXT NOT NULL, name TEXT NOT NULL, mime TEXT NOT NULL, uri TEXT, previous TEXT, wanted TEXT NOT NULL, ready INTEGER NOT NULL, siblings TEXT, UNIQUE(tree,parent,name))")
            execSQL("CREATE TABLE IF NOT EXISTS fingerprints (uri TEXT PRIMARY KEY, hash TEXT NOT NULL, stamp TEXT)")
            val hasStamp = rawQuery("PRAGMA table_info(fingerprints)", null).use { columns ->
                generateSequence { if (columns.moveToNext()) columns.getString(1) else null }.contains("stamp")
            }
            if (!hasStamp) execSQL("ALTER TABLE fingerprints ADD COLUMN stamp TEXT")
            execSQL("CREATE TABLE IF NOT EXISTS source_entries (uri TEXT PRIMARY KEY, tree TEXT NOT NULL, path TEXT NOT NULL, mime TEXT NOT NULL)")
        }
    }
    private fun baseline(uri: Uri): String? = mirrorDatabase.value.rawQuery(
        "SELECT hash FROM fingerprints WHERE uri=?", arrayOf(uri.toString())
    ).use { if (it.moveToFirst()) it.getString(0) else null }
    private fun rememberHash(uri: Uri, hash: ByteArray, stamp: String? = null) {
        mirrorDatabase.value.execSQL("INSERT INTO fingerprints(uri,hash,stamp) VALUES (?,?,?) ON CONFLICT(uri) DO UPDATE SET hash=excluded.hash,stamp=excluded.stamp WHERE hash!=excluded.hash OR stamp IS NOT excluded.stamp",
            arrayOf(uri.toString(), hashLabel(hash), stamp))
    }
    private fun hashLabel(hash: ByteArray?) = hash?.joinToString("") { "%02x".format(it) }
    private fun ensureOpen() { if (closed) throw CancellationException("Project session closed") }

    fun projectDirectory(): File {
        check(!closing) { "Project session closed" }
        return root
    }

    suspend fun openTree(tree: Uri, progress: (String) -> Unit): List<File> = withContext(Dispatchers.IO) {
        mutex.withLock {
            check(!closing) { "Project session closed" }
            val existing = origins.all.entries.firstOrNull { it.value == tree.toString() }?.key?.let(::File)
            val folder = existing ?: File(root, UUID.randomUUID().toString())
            try {
                restoreSources(tree, folder)
                reconcile(tree, folder, prepareWatch(tree), progress)
                val projects = folder.walkTopDown().onEnter { it.name != ".snapshots" }.filter { it.isFile && it.extension.equals("shrimp", true) }.toList()
                check(projects.isNotEmpty()) { "This folder does not contain a .shrimp project." }
                check(origins.edit().putString(folder.absolutePath, tree.toString()).commit()) { "Could not remember the project folder." }
                projects
            } catch (error: Exception) {
                cancelOpen()
                if (existing == null) folder.deleteRecursively()
                throw error
            }
        }
    }

    suspend fun openRequest(path: String, reconciled: Boolean = false): JSONObject = withContext(Dispatchers.IO) {
        mutex.withLock {
            check(!closing) { "Project session closed" }
            val folder = projectRoot(File(path))
            JSONObject().put("op", "open").put("path", path).apply {
                origins.getString(folder.absolutePath, null)?.let(Uri::parse)?.let { tree ->
                    if (!reconciled) {
                        restoreSources(tree, folder)
                        val openedId = sourceFiles.entries.firstOrNull { it.value.second == File(path) }?.key
                        reconcile(tree, folder, prepareWatch(tree)) {}
                        openedId?.let { id -> sourceFiles[id]?.second?.let { put("path", it.absolutePath) } }
                    }
                    put("mirror_root", folder.absolutePath)
                }
            }
        }
    }

    suspend fun exportTree(projectPath: String, tree: Uri, progress: (String) -> Unit) = withContext(Dispatchers.IO) {
        mutex.withLock {
            check(!closing) { "Project session closed" }
            val folder = projectRoot(File(projectPath))
            recoverUploads(tree)
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
            recoverUploads(destination)
            copyTo(folder, destination, DocumentsContract.getTreeDocumentId(destination), mime, progress)
        }
    }

    suspend fun importFile(source: Uri, consume: suspend (String, String, java.io.FileDescriptor) -> Unit) = withContext(Dispatchers.IO) {
        // Directory CREATE events can trigger recursive mirror scans. Keep those
        // scans behind the complete native import, including validation/rollback.
        mutex.withLock {
            check(!closing) { "Project session closed" }
            val name = resolver.query(source, arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME), android.os.Bundle.EMPTY, null)?.use {
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
                reconcile(tree, folder) {}
            }
        }
    }

    suspend fun sourcePath(uri: Uri): String? = withContext(Dispatchers.IO) {
        mutex.withLock {
            check(!closing) { "Project session closed" }
            ProjectDocumentsProvider.sourceFile(context, uri)?.absolutePath ?: run {
                val id = if (DocumentsContract.isDocumentUri(context, uri)) DocumentsContract.getDocumentId(uri) else DocumentsContract.getTreeDocumentId(uri)
                var mapped: String? = null
                for ((folder, origin) in origins.all) {
                    val tree = (origin as? String)?.let(Uri::parse) ?: continue
                    if (tree.authority != uri.authority) continue
                    if (DocumentsContract.getTreeDocumentId(tree) == id) { mapped = File(folder).canonicalPath; break }
                    val document = document(tree, id)
                    mirrorDatabase.value.rawQuery("SELECT path FROM source_entries WHERE uri=? LIMIT 1", arrayOf(document.toString())).use {
                        if (it.moveToFirst()) mapped = File(it.getString(0)).canonicalPath
                    }
                    if (mapped != null) break
                }
                mapped
            }
        }
    }

    suspend fun showInFiles(path: String) = withContext(Dispatchers.IO) {
        val directory = mutex.withLock {
            check(!closing) { "Project session closed" }
            val file = File(path).canonicalFile
            check(file.exists()) { "The selected file is unavailable." }
            val directory = if (file.isDirectory) file else file.parentFile!!
            val folder = if (directory == root.canonicalFile) directory else projectRoot(file)
            val tree = origins.getString(folder.absolutePath, null)?.let(Uri::parse)
            if (tree == null) ProjectDocumentsProvider.directoryUri(context, directory)
            else {
                val original = if (directory == folder) null else mirrorDatabase.value.rawQuery(
                    "SELECT uri FROM source_entries WHERE tree=? AND path=? AND mime=? LIMIT 1",
                    arrayOf(tree.toString(), directory.absolutePath, DocumentsContract.Document.MIME_TYPE_DIR)
                ).use { if (it.moveToFirst()) Uri.parse(it.getString(0)) else null }
                original ?: if (directory == folder) document(tree, DocumentsContract.getTreeDocumentId(tree))
                    else ProjectDocumentsProvider.directoryUri(context, directory)
            }
        }
        withContext(Dispatchers.Main) {
            try {
                context.startActivity(Intent(Intent.ACTION_VIEW).setDataAndType(directory, DocumentsContract.Document.MIME_TYPE_DIR)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PREFIX_URI_PERMISSION))
            } catch (failure: ActivityNotFoundException) {
                throw IllegalStateException("No installed file manager can display this project folder.", failure)
            }
        }
    }

    @Synchronized private fun prepareWatch(tree: Uri): DocumentTreeWatch {
        watch?.takeIf { it.tree == tree }?.let { return it }
        pendingWatch?.takeIf { it.tree == tree }?.let { return it }
        cancelOpen()
        return DocumentTreeWatch(resolver, tree).also { pendingWatch = it }
    }

    @Synchronized fun cancelOpen() {
        pendingWatch?.let { old -> old.close(); scope.launch { old.retain(emptySet()) } }
        pendingWatch = null
    }

    @Synchronized private fun startWatch(tree: Uri, project: File): DocumentTreeWatch {
        val active = prepareWatch(tree)
        pendingWatch = null
        active.project = project
        watch = active
        return active
    }

    private fun reconcile(tree: Uri, folder: File, active: DocumentTreeWatch? = watch?.takeIf { it.tree == tree }, progress: (String) -> Unit) {
        if (sourceTree != tree) restoreSources(tree, folder)
        resumeMove(tree)
        recoverUploads(tree)
        val changes = active?.changes()
        // Enumerate the whole topology before interpreting any missing path as a deletion:
        // a provider can keep an ID when moving a document between sibling directories.
        val listing = linkedMapOf<String, List<Entry>>()
        val ids = hashSetOf<String>()
        val rootId = DocumentsContract.getTreeDocumentId(tree)
        val pending = ArrayDeque(listOf(rootId))
        while (pending.isNotEmpty()) {
            val parent = pending.removeFirst()
            active?.observe(parent)
            val entries = entries(tree, parent).filterNot { ignored(it.name) }
            listing[parent] = entries
            for (entry in entries) {
                check(entry.id != rootId && ids.size < MAX_MIRRORED_ENTRIES && ids.add(entry.id)) {
                    "The provider returned repeated document IDs or exceeded the file budget."
                }
                if (entry.directory) pending.addLast(entry.id)
            }
        }
        active?.retain(listing.keys)
        val openDocument = active?.project?.toPath()
        val pinned = sourceFiles.entries.filter { openDocument?.startsWith(it.value.second.toPath()) == true }
            .associate { it.value.second to it.key }
        val locations = linkedMapOf<String, Pair<Entry, File>>()
        for ((parent, children) in listing) {
            val directory = locations[parent]?.second ?: folder
            for (entry in children) {
                val natural = File(directory, entry.name)
                val previous = sourceFiles[entry.id]?.second
                // Keep the editor's locked document at its original path through a
                // live rename. A new document ID replacing it in place keeps its name.
                val path = when {
                    previous != null && pinned[previous] == entry.id -> previous
                    pinned[natural] in ids -> File(folder, ".incoming-${UUID.nameUUIDFromBytes(entry.id.toByteArray())}")
                    else -> natural
                }
                locations[entry.id] = entry to path
            }
        }
        val byPath = locations.values.associate { (entry, file) -> file to entry }
        check(byPath.size == locations.size) { "Two source documents need the same local path. Close the project before retrying." }
        for ((id, location) in locations) {
            val (entry, target) = location
            val (previous, old) = sourceFiles[id] ?: continue
            check(previous.mime == entry.mime) { "'${entry.name}' changed file type externally. The local copy was kept." }
            if (old == target) continue
            if (target.exists()) {
                val displaced = sourceFiles.entries.firstOrNull { it.value.second == target }
                check(displaced != null) { "A local file already occupies '${target.name}'. Both copies were kept." }
                if (displaced.key !in locations) {
                    // Move surviving descendants first, even if the source removed their parent.
                    check(!displaced.value.first.directory || sourceFiles.none { (key, value) ->
                        key in locations && value.second.toPath().startsWith(target.toPath())
                    }) { "The source replaced a directory while moving its contents. The local files were kept." }
                    removeDeletedSource(tree, displaced.key, target)
                } else {
                    // The source names can form a cycle (a→b, b→a). Vacate the
                    // occupied path before moving the next document into it.
                    val stage = File(folder, ".incoming-${UUID.randomUUID()}")
                    move(tree, displaced.key, target, stage)
                }
            }
            move(tree, id, sourceFiles.getValue(id).second, target)
        }
        for ((id, old) in sourceFiles.toMap()) {
            if (id in locations || id !in sourceFiles) continue
            val (previous, file) = old
            val replacement = byPath[file]
            if (replacement == null && file in pinned) {
                if (file.isFile) check(file.inputStream().use { hashLabel(digest(it)) } == baseline(document(tree, id))) {
                    "'${file.name}' was removed externally. Export the project to preserve your local edits."
                }
                continue
            }
            if (replacement == null) removeDeletedSource(tree, id, file)
            else {
                check(replacement.mime == previous.mime) { "'${replacement.name}' changed file type externally. The local copy was kept." }
                val oldUri = document(tree, id).toString()
                val newUri = document(tree, replacement.id).toString()
                mirrorDatabase.value.transaction {
                    execSQL("INSERT INTO fingerprints(uri,hash) SELECT ?,hash FROM fingerprints WHERE uri=? ON CONFLICT(uri) DO UPDATE SET hash=excluded.hash,stamp=NULL", arrayOf(newUri, oldUri))
                    forgetEntry(oldUri)
                    rememberEntry(tree, replacement, file)
                }
                sourceFiles.remove(id)
            }
        }
        val deleted = if (folder.isDirectory) deleteMissingSources(tree, folder) else emptySet()
        for ((parent, entries) in listing) {
            if (parent in deleted) continue
            val directory = locations[parent]?.second ?: folder
            check(directory.isDirectory || directory.mkdirs()) { "Could not create the project folder." }
            for (entry in entries) {
                ensureOpen()
                if (entry.id in deleted) continue
                val target = locations.getValue(entry.id).second
                if (!entry.directory) {
                    val forced = changes?.directories?.let { parent in it || "" in it } == true
                    syncFile(tree, parent, entry, target, forced, changes?.checkUnversioned != false, progress)
                }
                rememberEntry(tree, entry, target)
            }
        }
        // The complete listing already reconciled every tracked file. Publish local additions only.
        val tracked = sourceFiles.values.mapTo(HashSet()) { it.second }
        folder.walkTopDown().onEnter { !ignored(it.name) }.filter { it != folder && it !in tracked && !ignored(it.name) }.forEach {
            if (it.isDirectory) destinationDirectory(folder, it, tree) else writeChangedFile(folder, it, tree)
        }
        if (changes != null) active.acknowledge(changes)
    }

    // Atomically move the local inode and all persisted descendant paths, including
    // recovery after a process exit between the filesystem rename and SQLite commit.
    private fun move(tree: Uri, id: String, source: File, target: File) {
        check(source.exists()) { "'${source.name}' was removed locally and moved externally. The source was kept." }
        check(!target.exists()) { "The local path '${target.name}' is occupied." }
        mirrorDatabase.value.execSQL("INSERT INTO relocations(tree,document,source,target,inode) VALUES(?,?,?,?,?)",
            arrayOf(tree.toString(), id, source.path, target.path, inode(source)))
        resumeMove(tree)
    }

    private fun inode(file: File): String? = if (!file.exists()) null else android.system.Os.stat(file.path).let {
        "${it.st_dev}:${it.st_ino}"
    }

    private fun resumeMove(tree: Uri) {
        mirrorDatabase.value.rawQuery("SELECT document,source,target,inode FROM relocations WHERE tree=?", arrayOf(tree.toString())).use { row ->
            if (!row.moveToFirst()) return
            val id = row.getString(0)
            val source = File(row.getString(1))
            val target = File(row.getString(2))
            val expected = row.getString(3)
            if (inode(target) != expected) {
                check(inode(source) == expected && !target.exists()) { "A pending project move conflicts with local changes. Both copies were kept." }
                val parent = checkNotNull(target.parentFile)
                check(parent.isDirectory || parent.mkdirs()) { "Could not create the new parent directory." }
                Files.move(source.toPath(), target.toPath(), StandardCopyOption.ATOMIC_MOVE)
            }
            val relocated = sourceFiles.filter { (key, value) -> key == id ||
                target.isDirectory && value.second.toPath().startsWith(source.toPath())
            }.mapValues { (_, pair) ->
                val (entry, path) = pair
                entry to if (path == source) target else File(target, path.relativeTo(source).path)
            }
            mirrorDatabase.value.transaction {
                for ((key, pair) in relocated) execSQL("UPDATE source_entries SET path=? WHERE uri=?",
                    arrayOf(pair.second.path, document(tree, key).toString()))
                delete("relocations", "tree=?", arrayOf(tree.toString()))
            }
            sourceFiles.putAll(relocated)
        }
    }

    fun resumed() { watch?.resumed() }

    fun localChanged(root: String, relativePath: String) { localChange?.invoke(root, relativePath) }

    fun attach(projectPath: String?, error: (String) -> Unit): Int {
        detach()
        check(!closing) { "Project session closed" }
        onError = error
        if (projectPath == null) return generation
        val project = File(projectPath)
        val folder = projectRoot(project)
        val tree = origins.getString(folder.absolutePath, null)?.let(Uri::parse) ?: return generation
        val epoch = generation
        val active = startWatch(tree, project)
        localChange = { root, _ -> if (root == folder.absolutePath) active.localChanged() }
        scope.launch {
            try {
                while (active.awaitChange()) {
                    try {
                        mutex.withLock {
                            if (generation != epoch) return@launch
                            reconcile(tree, folder) {}
                        }
                    } catch (failure: Exception) { report(epoch, failure) }
                }
            } finally { active.retain(emptySet()) }
        }
        return epoch
    }

    private fun destinationDirectory(root: File, directory: File, tree: Uri): String {
        if (directory == root) return DocumentsContract.getTreeDocumentId(tree)
        if (tree == sourceTree) {
            sourceFiles.entries.firstOrNull { it.value.second == directory && it.value.first.directory }?.let { return it.key }
        }
        val parent = destinationDirectory(root, checkNotNull(directory.parentFile), tree)
        val existing = entries(tree, parent).firstOrNull { it.name == directory.name }
        val id = destinationSubdirectory(tree, parent, directory.name, existing)
        if (tree == sourceTree) rememberEntry(tree, Entry(id, directory.name, DocumentsContract.Document.MIME_TYPE_DIR), directory)
        return id
    }
    private fun destinationSubdirectory(tree: Uri, parent: String, name: String, existing: Entry?): String {
        if (existing != null) {
            check(existing.mime == DocumentsContract.Document.MIME_TYPE_DIR) { "'$name' is a file in the destination folder." }
            return existing.id
        }
        val uri = DocumentsContract.createDocument(resolver, document(tree, parent), DocumentsContract.Document.MIME_TYPE_DIR, name)
            ?: error("Could not create '$name'.")
        val id = DocumentsContract.getDocumentId(uri)
        val created = inspect(tree, id)
        if (created?.name != name || created.mime != DocumentsContract.Document.MIME_TYPE_DIR) {
            if (created?.mime == DocumentsContract.Document.MIME_TYPE_DIR && entries(tree, id).isEmpty()) {
                check(DocumentsContract.deleteDocument(resolver, uri)) { "Could not remove the directory the provider named '${created.name}'." }
            }
            error("The provider cannot preserve the directory name '$name'.")
        }
        return id
    }
    private fun writeChangedFile(root: File, file: File, tree: Uri) {
        val parent = destinationDirectory(root, file.parentFile!!, tree)
        writeFile(file, tree, parent, entries(tree, parent).firstOrNull { it.name == file.name })
    }

    private fun report(epoch: Int, failure: Exception) {
        main.post { if (epoch == generation) onError("Could not synchronize project: ${failure.message}") }
    }

    private fun projectRoot(file: File): File {
        val canonicalRoot = root.canonicalFile
        val canonical = file.canonicalFile
        return generateSequence(if (canonical.isDirectory) canonical else canonical.parentFile) { it.parentFile }
            .firstOrNull { it.parentFile == canonicalRoot }
            ?: throw IllegalArgumentException("Project is outside Android's managed project directory.")
    }

    // Provider metadata is an optional freshness hint. Document identity comparisons
    // and the deletion preflight intentionally compare only the ID, name, and MIME.
    private data class Entry(val id: String, val name: String, val mime: String) {
        val directory get() = mime == DocumentsContract.Document.MIME_TYPE_DIR
        var size: Long? = null
        var modified: Long? = null
    }
    private fun document(tree: Uri, id: String) = DocumentsContract.buildDocumentUriUsingTree(tree, id)
    private fun forgetEntry(uri: String) {
        for (table in listOf("fingerprints", "source_entries")) {
            mirrorDatabase.value.delete(table, "uri=?", arrayOf(uri))
        }
    }
    private fun rememberEntry(tree: Uri, entry: Entry, file: File) {
        check(sourceFiles.containsKey(entry.id) || sourceFiles.size < MAX_MIRRORED_ENTRIES) { "The project folder exceeds its file budget." }
        val location = entry to file
        if (sourceFiles[entry.id] == location) return
        mirrorDatabase.value.execSQL("INSERT INTO source_entries(uri,tree,path,mime) VALUES (?,?,?,?) ON CONFLICT(uri) DO UPDATE SET tree=excluded.tree,path=excluded.path,mime=excluded.mime WHERE tree!=excluded.tree OR path!=excluded.path OR mime!=excluded.mime",
            arrayOf(document(tree, entry.id).toString(), tree.toString(), file.absolutePath, entry.mime))
        sourceFiles[entry.id] = location
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

    private fun entries(tree: Uri, parent: String) =
        queryEntries(DocumentsContract.buildChildDocumentsUriUsingTree(tree, parent))

    private fun queryEntries(uri: Uri): List<Entry> {
        fun query() = resolver.query(uri, arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_SIZE, DocumentsContract.Document.COLUMN_LAST_MODIFIED), android.os.Bundle.EMPTY, null)
            ?: error("The document provider could not list the project folder.")
        fun complete(cursor: android.database.Cursor): List<Entry>? {
            cursor.extras.getString(DocumentsContract.EXTRA_ERROR)?.let { error(it) }
            if (cursor.extras.getBoolean(DocumentsContract.EXTRA_LOADING)) return null
            val names = mutableSetOf<String>()
            return buildList { while (cursor.moveToNext()) {
                check(size < MAX_MIRRORED_ENTRIES) { "The provider folder exceeds its file budget." }
                val entry = Entry(cursor.getString(0), cursor.getString(1), cursor.getString(2))
                fun optionalLong(column: String): Long? {
                    val index = cursor.getColumnIndex(column)
                    return if (index >= 0 && !cursor.isNull(index)) cursor.getLong(index) else null
                }
                entry.size = optionalLong(DocumentsContract.Document.COLUMN_SIZE)
                entry.modified = optionalLong(DocumentsContract.Document.COLUMN_LAST_MODIFIED)
                checkName(entry.name)
                check(names.add(entry.name)) { "The provider returned duplicate file names." }
                add(entry)
            } }
        }
        while (true) {
            query().use { cursor ->
                complete(cursor)?.let { return it }
                val changed = CountDownLatch(1)
                val observer = object : ContentObserver(null) {
                    override fun onChange(selfChange: Boolean) = changed.countDown()
                }
                cursor.registerContentObserver(observer)
                try {
                    // Close the race between the initial query and observer registration.
                    query().use { complete(it) }?.let { return it }
                    check(changed.await(30, TimeUnit.SECONDS)) { "The document provider is still loading. Retry when the folder is available." }
                    check(!closing) { "Project session closed" }
                } finally { cursor.unregisterContentObserver(observer) }
            }
        }
    }

    private fun inspect(tree: Uri, id: String): Entry? {
        val result = queryEntries(document(tree, id))
        check(result.size <= 1) { "The provider returned multiple entries for one document." }
        return result.firstOrNull()?.also { check(it.id == id) { "The provider returned a different document." } }
    }

    private fun checkName(name: String) {
        require(name.isNotBlank() && name != "." && name != ".." && !name.contains('/') && !name.contains('\\')) {
            "The document provider returned an invalid file name."
        }
    }
    private fun ignored(name: String) = PRIVATE_FILE.matches(name)

    private fun removeDeletedSource(tree: Uri, id: String, target: File) {
        // A remote deletion must not erase newer or newly created local descendants.
        val known = if (target.isDirectory) sourceFiles.entries.associate { it.value.second.absolutePath to it.key } else emptyMap()
        for (file in target.walkTopDown().filter { it.isFile && !ignored(it.name) }) {
            val sourceId = if (file == target) id else known[file.absolutePath]
            val base = sourceId?.let { baseline(document(tree, it)) }
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
        mirrorDatabase.value.transaction {
            for (sourceId in removed) forgetEntry(document(tree, sourceId).toString())
        }
        sourceFiles.keys.removeAll(removed.toSet())
    }

    private fun deleteMissingSources(tree: Uri, folder: File): Set<String> {
        val directories = sourceFiles.values.filter { it.first.mime == DocumentsContract.Document.MIME_TYPE_DIR }.associate { it.second to it.first }
        val absent = sourceFiles.filter { (_, value) ->
            !value.second.exists()
        }
        // Older mirrors registered incoming entries before copying their bytes. An
        // unverified descendant makes its whole missing subtree a download, never a deletion.
        val incomplete = absent.mapNotNull { (id, value) ->
            val (entry, file) = value
            val missingReceipt = entry.mime != DocumentsContract.Document.MIME_TYPE_DIR &&
                baseline(document(tree, id)) == null
            val parents = generateSequence(file.parentFile) { it.parentFile }.takeWhile { it != folder }
            if (missingReceipt || parents.any { it !in directories }) {
                generateSequence(file) { it.parentFile }.takeWhile { it != folder && !it.exists() }.last()
            } else null
        }.toSet()
        val affected = absent.filterValues { (_, file) -> generateSequence(file) { it.parentFile }.none { it in incomplete } }
        val paths = affected.values.map { it.second }.toSet()
        val roots = affected.values.filter { (_, file) -> generateSequence(file.parentFile) { it.parentFile }.none { it in paths } }
        val parentListings = mutableMapOf<String, Map<String, Entry>>()
        val alreadyGone = mutableSetOf<String>()
        fun parentId(file: File) = if (file == folder) DocumentsContract.getTreeDocumentId(tree)
            else directories[file]?.id ?: error("The source parent is not tracked. Its contents were kept.")
        // Preflight every descendant before deleting a provider directory. Unknown,
        // renamed or externally modified children preserve the entire remote subtree.
        for (root in roots) {
            val pending = ArrayDeque<Pair<Entry, File>>()
            pending.add(root)
            while (pending.isNotEmpty()) {
                val (expected, file) = pending.removeFirst()
                val current = inspect(tree, expected.id)
                if (current == null) {
                    check(file == root.second && entries(tree, parentId(requireNotNull(file.parentFile))).none { it.id == expected.id }) {
                        "The provider still lists '${expected.name}' but could not inspect it. Its contents were kept."
                    }
                    alreadyGone.add(expected.id)
                    continue
                }
                check(current == expected) { "'${expected.name}' changed externally while being removed locally. The source was kept." }
                if (file == root.second) {
                    var ancestor = file
                    var identity = expected
                    while (ancestor != folder) {
                        val parent = ancestor.parentFile ?: error("The mirrored path has no parent.")
                        val parentId = parentId(parent)
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
                    val uri = document(tree, current.id)
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
                check(if (entry.id in alreadyGone) current == null else current == entry) { "'${entry.name}' changed externally during deletion. The source was kept." }
                if (current != null) {
                    check(DocumentsContract.deleteDocument(resolver, document(tree, entry.id))) { "Could not delete source '${entry.name}'." }
                }
                deleted.add(target)
            }
        } finally {
            // One metadata scan/transaction for the successful subset, including a
            // partially failed batch. Do not forget provider deletions that failed.
            if (deleted.isNotEmpty()) forgetSources(tree, deleted)
        }
        return affected.keys
    }

    private fun localStamp(file: File): String? {
        if (!file.isFile) return null
        val stat = android.system.Os.stat(file.path)
        return "${stat.st_dev}:${stat.st_ino}:${stat.st_size}:${stat.st_mtim.tv_sec}.${stat.st_mtim.tv_nsec}:${stat.st_ctim.tv_sec}.${stat.st_ctim.tv_nsec}"
    }

    // A stamp only skips unchanged reads. Actual publication or deletion still checks the
    // SHA-256 baseline against both sides. Unknown provider metadata is reverified on open
    // and on directory invalidation, even when there is no useful modification time.
    private fun syncFile(tree: Uri, parent: String, entry: Entry, target: File, forced: Boolean, checkUnversioned: Boolean, progress: (String) -> Unit) {
        val uri = document(tree, entry.id)
        val before = localStamp(target)
        val size = entry.size ?: -1L
        val modified = entry.modified ?: 0L
        val reliable = size >= 0 && modified > 0
        val recent = modified >= System.currentTimeMillis() - PROVIDER_TIME_RESOLUTION_MS
        val remoteStamp = "$size:$modified"
        val stamp = before?.let { "$it:$remoteStamp" }
        if (!forced && stamp != null && (!checkUnversioned || reliable && !recent) && mirrorDatabase.value.rawQuery(
                "SELECT 1 FROM fingerprints WHERE uri=? AND stamp=?", arrayOf(uri.toString(), stamp)
            ).use { it.moveToFirst() }) return
        progress(entry.name)
        val current = inspect(tree, entry.id) ?: throw java.io.FileNotFoundException(entry.name)
        check(current == entry) { "'${entry.name}' changed externally. Retry synchronization." }
        val remoteHash = resolver.openInputStream(uri)?.use { digest(it) }
            ?: throw IllegalArgumentException("Could not read '${entry.name}'.")
        val localHash = target.takeIf(File::isFile)?.inputStream()?.use { digest(it) }
        if (remoteHash.contentEquals(localHash)) {
            rememberHash(uri, remoteHash, stamp)
            return
        }
        val base = baseline(uri)
        if (localHash != null && hashLabel(localHash) != base) {
            check(base != null && hashLabel(remoteHash) == base) { "'${entry.name}' has different changes on this device and in its source folder. Export the local project before reopening its source." }
            writeFile(target, tree, parent, entry)
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
            val staged = checkNotNull(localStamp(incoming)).substringBeforeLast(':')
            Files.move(incoming.toPath(), target.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
            // Keep the fast path tied to the verified inode even if Rust replaces
            // the destination immediately after publication.
            val installed = localStamp(target)
            rememberHash(uri, remoteHash, installed?.takeIf { it.substringBeforeLast(':') == staged }?.let { "$it:$remoteStamp" })
        } finally { incoming.delete() }
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
                val id = destinationSubdirectory(tree, parent, file.name, existing)
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
            val wanted = checkNotNull(hashLabel(localHash))
            input.channel.position(0)
            check(tree != sourceTree || existing != null || sourceFiles.size < MAX_MIRRORED_ENTRIES) { "The project folder exceeds its file budget." }
            val uri = existing?.let { document(tree, it.id) }
            var previous: String? = null
            if (uri != null) {
                previous = resolver.openInputStream(uri)?.use { hashLabel(digest(it)) }
                    ?: error("Could not read destination '${file.name}'.")
                if (previous == wanted) {
                    if (tree == sourceTree) rememberEntry(tree, existing, file)
                    rememberHash(uri, localHash)
                    return
                }
                val base = baseline(uri)
                check(base != null && (previous == base || wanted == base)) {
                    "'${file.name}' has conflicting changes. Export to an empty folder to preserve both versions."
                }
                if (wanted == base) {
                    check(tree == sourceTree) { "The export destination has newer content. Choose an empty folder." }
                    syncFile(tree, parent, existing, file, forced = true, checkUnversioned = true) {}
                    return
                }
            }
            // SAF provides no atomic replacement primitive for an existing document URI.
            // Keep the exact intended replacement in private durable storage before truncating.
            val stage = File(context.getDir("uploads", Context.MODE_PRIVATE), UUID.randomUUID().toString())
            val upload = Upload(stage, parent, existing?.name ?: file.name, existing?.mime ?: mime, uri, previous, wanted)
            mirrorDatabase.value.execSQL("INSERT INTO uploads(stage,tree,parent,name,mime,uri,previous,wanted,ready) VALUES(?,?,?,?,?,?,?,?,?)",
                arrayOf<Any?>(stage.name, tree.toString(), parent, upload.name, upload.mime, uri?.toString(), previous, wanted, UPLOAD_CLEANUP))
            try {
                saveStage(input, stage, wanted)
                if (uri != null) {
                    checkNotNull(resolver.openInputStream(uri)).use {
                        saveStage(it, originalStage(stage), checkNotNull(previous))
                    }
                }
                setUploadPhase(stage, UPLOAD_STAGED)
            } catch (failure: Exception) {
                removeStage(stage)
                throw failure
            }
            val destination = finishUpload(tree, upload, UPLOAD_STAGED)
            if (tree == sourceTree) rememberEntry(tree, Entry(DocumentsContract.getDocumentId(destination), upload.name, existing?.mime ?: mime), file)
        }
    }

    private data class Upload(val stage: File, val parent: String, val name: String, val mime: String,
                              val original: Uri?, val previous: String?, val wanted: String)

    private fun setUploadPhase(stage: File, phase: Int) {
        mirrorDatabase.value.execSQL("UPDATE uploads SET ready=? WHERE stage=?", arrayOf<Any>(phase, stage.name))
    }

    private fun recoverUploads(tree: Uri) {
        mirrorDatabase.value.rawQuery("SELECT stage,parent,name,mime,uri,previous,wanted,ready FROM uploads WHERE tree=?",
            arrayOf(tree.toString())).use { cursor ->
            while (cursor.moveToNext()) {
                val file = File(context.getDir("uploads", Context.MODE_PRIVATE), cursor.getString(0))
                val phase = cursor.getInt(7)
                if (phase == UPLOAD_CLEANUP) removeStage(file)
                else finishUpload(tree, Upload(file, cursor.getString(1), cursor.getString(2), cursor.getString(3),
                    cursor.getString(4)?.let(Uri::parse), cursor.getString(5), cursor.getString(6)), phase)
            }
        }
    }

    private fun originalStage(stage: File) = File(stage.parentFile, "${stage.name}.original")

    private fun saveStage(input: java.io.InputStream, stage: File, expected: String) {
        stage.outputStream().use { output ->
            val hash = MessageDigest.getInstance("SHA-256")
            copy(java.security.DigestInputStream(input, hash), output)
            check(hashLabel(hash.digest()) == expected) { "The file changed while preparing the upload." }
            output.fd.sync()
        }
    }

    private fun removeStage(stage: File) {
        for (part in listOf(stage, originalStage(stage))) {
            check(!part.exists() || part.delete()) { "Could not clean up the finished upload." }
        }
        mirrorDatabase.value.execSQL("DELETE FROM uploads WHERE stage=?", arrayOf(stage.name))
    }

    private fun finishUpload(tree: Uri, upload: Upload, phase: Int): Uri {
        val (stage, parent, name, mime, original, previous, wanted) = upload
        val listing = entries(tree, parent)
        val entry = listing.firstOrNull { it.name == name }
        val uri = if (entry != null) {
            document(tree, entry.id).also {
                check(original == null || original == it) { "'$name' was replaced externally during upload. The staged version is preserved." }
            }
        } else {
            check(original == null) { "The provider moved, renamed or omitted '$name' during upload. The staged version is preserved." }
            // createDocument assigns the ID. If the process dies before it returns, only
            // retry creation when the directory still has exactly the same document IDs.
            val siblings = hashLabel(digest(org.json.JSONArray(listing.map(Entry::id).sorted()).toString().byteInputStream()))
            if (phase == UPLOAD_CREATING) {
                val unchanged = mirrorDatabase.value.rawQuery("SELECT siblings FROM uploads WHERE stage=?", arrayOf(stage.name)).use {
                    it.moveToFirst() && it.getString(0) == siblings
                }
                check(unchanged) { "The provider changed the folder while creating '$name'. The staged version is preserved." }
            }
            mirrorDatabase.value.execSQL("UPDATE uploads SET ready=?,siblings=? WHERE stage=?", arrayOf<Any?>(UPLOAD_CREATING, siblings, stage.name))
            DocumentsContract.createDocument(resolver, document(tree, parent), mime, name)
                ?: error("Could not create '$name'.")
        }
        if (original == null) mirrorDatabase.value.execSQL("UPDATE uploads SET uri=? WHERE stage=?", arrayOf(uri.toString(), stage.name))
        val actual = inspect(tree, DocumentsContract.getDocumentId(uri))
        if (actual?.name != name || actual.mime == DocumentsContract.Document.MIME_TYPE_DIR) {
            if (entry == null && actual != null && actual.mime != DocumentsContract.Document.MIME_TYPE_DIR &&
                resolver.openInputStream(uri)?.use { it.read() == -1 } == true && DocumentsContract.deleteDocument(resolver, uri)) {
                setUploadPhase(stage, UPLOAD_CLEANUP)
                removeStage(stage)
            }
            error("The provider cannot preserve the name '$name'. The local file is unchanged.")
        }
        fun readRemote() = resolver.openInputStream(uri)?.use { fingerprint(it) } ?: error("Could not read '$name'.")
        fun matchesPrefix(file: File, length: Long, hash: ByteArray): Boolean {
            val (count, prefix) = file.inputStream().use { fingerprint(it, length) }
            return count == length && prefix.contentEquals(hash)
        }
        fun publish(file: File, hash: String): ByteArray {
            check(file.inputStream().use { hashLabel(digest(it)) } == hash) { "The saved recovery copy of '$name' is damaged." }
            file.inputStream().use { input ->
                checkNotNull(resolver.openOutputStream(uri, "rwt")) { "Could not write '$name'." }.use { copy(input, it) }
            }
            return readRemote().second.also {
                check(hashLabel(it) == hash) { "Could not verify '$name' after upload. Retry to finish saving." }
            }
        }
        val (length, before) = readRemote()
        var remote = before
        if (hashLabel(remote) != wanted) {
            val backup = originalStage(stage)
            if (hashLabel(remote) != previous && !(previous == null && length == 0L)) {
                check((phase == UPLOAD_WRITING || phase == UPLOAD_RESTORING) &&
                    (matchesPrefix(stage, length, remote) || phase == UPLOAD_RESTORING && matchesPrefix(backup, length, remote))) {
                    "'$name' changed externally during upload. Both versions are preserved."
                }
            }
            setUploadPhase(stage, UPLOAD_WRITING)
            try {
                remote = publish(stage, wanted)
            } catch (failure: Exception) {
                if (previous != null) {
                    // Restore an interrupted prefix while the provider is still accessible.
                    // A repeated failure leaves both verified copies and the receipt on disk.
                    try {
                        val (size, hash) = readRemote()
                        if (hashLabel(hash) != wanted && matchesPrefix(stage, size, hash)) {
                            setUploadPhase(stage, UPLOAD_RESTORING)
                            publish(backup, previous)
                            setUploadPhase(stage, UPLOAD_STAGED)
                        }
                    } catch (restore: Exception) { failure.addSuppressed(restore) }
                }
                throw failure
            }
        }
        rememberHash(uri, remote)
        // Mark complete first so recovery can finish cleanup even if the process dies here.
        setUploadPhase(stage, UPLOAD_CLEANUP)
        removeStage(stage)
        return uri
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

    private fun digest(input: java.io.InputStream) = fingerprint(input).second

    private fun fingerprint(input: java.io.InputStream, limit: Long = Long.MAX_VALUE): Pair<Long, ByteArray> {
        val hash = MessageDigest.getInstance("SHA-256")
        val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
        var size = 0L
        while (size < limit) {
            ensureOpen()
            val count = input.read(buffer, 0, minOf(buffer.size.toLong(), limit - size).toInt())
            if (count < 0) break
            hash.update(buffer, 0, count)
            size += count
        }
        return size to hash.digest()
    }

    fun detach(owner: Int? = null) {
        if (owner != null && owner != generation) return
        generation++
        watch?.close()
        watch = null
        localChange = null
    }
    override fun close() {
        if (closing) return
        closing = true
        cancelOpen()
        detach()
        onError = {}
        scope.launch {
            // An output descriptor may already have truncated a provider document.
            // Finish its active transfer before shutdown can make copy() abort.
            mutex.withLock {
                closed = true
                if (mirrorDatabase.isInitialized()) mirrorDatabase.value.close()
            }
        }
        jobs.complete()
    }
}
