package dev.shrimply.app

import android.content.ContentResolver
import android.content.Intent
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import org.json.JSONObject

@Composable
internal fun rememberExportPicker(selected: (Uri?) -> Unit): (JSONObject) -> Unit {
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        selected(result.data?.data?.takeIf { result.resultCode == android.app.Activity.RESULT_OK })
    }
    return { destination ->
        val intent = if (destination.optBoolean("folder")) Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
        else Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
            .setType(destination.getString("mime")).putExtra(Intent.EXTRA_TITLE, destination.getString("filename"))
        launcher.launch(intent)
    }
}

internal class ExportDocument(private val resolver: ContentResolver, private val uri: Uri, private val created: Boolean) {
    private var descriptor: ParcelFileDescriptor? = null

    fun name(): String {
        return resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        } ?: uri.lastPathSegment.orEmpty()
    }

    fun open(): Int {
        val file = checkNotNull(resolver.openFileDescriptor(uri, "wt")) { "Could not open the export destination." }
        descriptor = file
        return file.fd
    }

    fun finish(success: Boolean) {
        if (success) {
            descriptor?.close()
        } else {
            try {
                descriptor?.closeWithError("Export did not finish.")
            } finally {
                if (created) check(DocumentsContract.deleteDocument(resolver, uri)) { "Could not remove the incomplete export." }
            }
        }
    }
}
