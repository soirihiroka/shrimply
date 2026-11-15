package dev.shrimply.app

import android.content.Context
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.CancellationSignal
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.provider.DocumentsContract.Root
import android.provider.DocumentsProvider
import android.webkit.MimeTypeMap
import java.io.File
import java.io.FileNotFoundException
import java.util.Locale

/** Read-only access to managed project files through the system document browser. */
class ProjectDocumentsProvider : DocumentsProvider() {
    private lateinit var root: File

    companion object {
        private const val ROOT_ID = "projects"
        private val ROOT_COLUMNS = arrayOf(Root.COLUMN_ROOT_ID, Root.COLUMN_DOCUMENT_ID,
            Root.COLUMN_TITLE, Root.COLUMN_FLAGS, Root.COLUMN_ICON, Root.COLUMN_MIME_TYPES,
            Root.COLUMN_AVAILABLE_BYTES)
        private val DOCUMENT_COLUMNS = arrayOf(Document.COLUMN_DOCUMENT_ID,
            Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE, Document.COLUMN_FLAGS,
            Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED)

        private fun documentId(root: File, file: File): String =
            if (file == root) ROOT_ID else "$ROOT_ID/${file.relativeTo(root).invariantSeparatorsPath}"

        internal fun directoryUri(context: Context, directory: File): Uri {
            val root = File(context.filesDir, "projects").canonicalFile
            val folder = directory.canonicalFile
            if (!folder.isDirectory || !folder.toPath().startsWith(root.toPath())) {
                throw FileNotFoundException("The managed project folder is unavailable.")
            }
            val id = documentId(root, folder)
            val tree = DocumentsContract.buildTreeDocumentUri(context.packageName + ".projects", id)
            return DocumentsContract.buildDocumentUriUsingTree(tree, id)
        }
    }

    override fun onCreate(): Boolean {
        root = File(requireNotNull(context).filesDir, "projects").canonicalFile
        check(root.isDirectory || root.mkdirs()) { "Could not open the managed project directory." }
        return true
    }

    private fun file(id: String): File {
        val target = when {
            id == ROOT_ID -> root
            id.startsWith("$ROOT_ID/") -> File(root, id.removePrefix("$ROOT_ID/"))
            else -> throw FileNotFoundException("Unknown project document.")
        }.canonicalFile
        if (!target.toPath().startsWith(root.toPath()) || documentId(root, target) != id
            || (!target.isFile && !target.isDirectory)) {
            throw FileNotFoundException("The project document is unavailable.")
        }
        return target
    }

    override fun queryRoots(projection: Array<out String>?): Cursor {
        val context = requireNotNull(context)
        val columns = projection ?: ROOT_COLUMNS
        return MatrixCursor(columns).apply {
            val row = newRow()
            for (column in columns) row.add(when (column) {
                Root.COLUMN_ROOT_ID, Root.COLUMN_DOCUMENT_ID -> ROOT_ID
                Root.COLUMN_TITLE -> context.applicationInfo.loadLabel(context.packageManager)
                Root.COLUMN_FLAGS -> Root.FLAG_LOCAL_ONLY or Root.FLAG_SUPPORTS_IS_CHILD
                Root.COLUMN_ICON -> context.applicationInfo.icon
                Root.COLUMN_MIME_TYPES -> "*/*"
                Root.COLUMN_AVAILABLE_BYTES -> root.usableSpace
                else -> null
            })
        }
    }

    private fun MatrixCursor.addDocument(file: File) {
        val row = newRow()
        for (column in columnNames) row.add(when (column) {
            Document.COLUMN_DOCUMENT_ID -> documentId(root, file)
            Document.COLUMN_DISPLAY_NAME -> if (file == root)
                requireNotNull(context).applicationInfo.loadLabel(requireNotNull(context).packageManager)
                else file.name
            Document.COLUMN_MIME_TYPE -> if (file.isDirectory) Document.MIME_TYPE_DIR else
                MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase(Locale.ROOT)) ?: "application/octet-stream"
            Document.COLUMN_FLAGS -> 0
            Document.COLUMN_SIZE -> if (file.isFile) file.length() else null
            Document.COLUMN_LAST_MODIFIED -> file.lastModified()
            else -> null
        })
    }

    override fun queryDocument(documentId: String, projection: Array<out String>?): Cursor =
        MatrixCursor(projection ?: DOCUMENT_COLUMNS).apply { addDocument(file(documentId)) }

    override fun queryChildDocuments(parentDocumentId: String, projection: Array<out String>?, sortOrder: String?): Cursor =
        MatrixCursor(projection ?: DOCUMENT_COLUMNS).apply {
            val children = file(parentDocumentId).listFiles()
                ?: throw FileNotFoundException("The project directory is unavailable.")
            for (child in children.sortedBy { it.name }) {
                val canonical = child.canonicalFile
                if (child == canonical && canonical.toPath().startsWith(root.toPath())
                    && (canonical.isFile || canonical.isDirectory)) {
                    addDocument(canonical)
                }
            }
        }

    override fun isChildDocument(parentDocumentId: String, documentId: String): Boolean {
        val parent = file(parentDocumentId)
        val child = file(documentId)
        return parent.isDirectory && child != parent && child.toPath().startsWith(parent.toPath())
    }

    override fun findDocumentPath(parentDocumentId: String?, childDocumentId: String): DocumentsContract.Path {
        val parent = parentDocumentId?.let(::file) ?: root
        val child = file(childDocumentId)
        if (!parent.isDirectory || !child.toPath().startsWith(parent.toPath())) {
            throw FileNotFoundException("The project document is outside the requested directory.")
        }
        val path = generateSequence(child) { it.parentFile }.takeWhile { it != parent }
            .map { documentId(root, it) }.toList().asReversed()
        return DocumentsContract.Path(if (parentDocumentId == null) ROOT_ID else null,
            listOf(documentId(root, parent)) + path)
    }

    override fun openDocument(documentId: String, mode: String, signal: CancellationSignal?): ParcelFileDescriptor {
        if (mode != "r") throw UnsupportedOperationException("Managed project documents are read-only.")
        signal?.throwIfCanceled()
        val document = file(documentId)
        if (!document.isFile) throw FileNotFoundException("A project directory cannot be opened as a file.")
        return ParcelFileDescriptor.open(document, ParcelFileDescriptor.MODE_READ_ONLY)
    }
}
