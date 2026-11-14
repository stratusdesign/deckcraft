package ai.storyteller.deckcraft;

import android.app.NativeActivity;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.provider.DocumentsContract;
import android.provider.OpenableColumns;

import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * DeckCraft's window (a NativeActivity running the Rust app) plus what Rust can't do alone on
 * Android: reach the user's folders. Adding a location asks the system folder picker, whose
 * answer only reaches a Java activity (nativePicked); after that Rust lists folders, makes files,
 * and copies documents in and out of their working copies through the methods below.
 */
public class DeckcraftActivity extends NativeActivity {
    static {
        // Loaded here as well as by NativeActivity, so the JVM can find nativePicked.
        System.loadLibrary("deckcraft_android");
    }

    static final int TREE = 3;
    /** A document ChromeOS or another app asked us to open. */
    static final int VIEW = 4;
    static final int VIEW_READ_ONLY = 5;
    static final int RW = Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION;

    static native void nativePicked(int kind, String uri, String name);

    /** The MIME type a new file gets, from its extension. */
    static String mimeFor(String name) {
        int dot = name.lastIndexOf('.');
        String ext = dot < 0 ? "" : name.substring(dot + 1).toLowerCase();
        String mime = android.webkit.MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext);
        return mime == null ? "application/octet-stream" : mime;
    }

    /** Add a location: the user picks a folder DeckCraft may use (My files, Drive, …). */
    /** The text on the clipboard, "" when it holds none (Rust: clipboard::get). */
    public String clipboardText() {
        android.content.ClipboardManager cm = (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        if (cm == null) return "";
        android.content.ClipData clip = cm.getPrimaryClip();
        if (clip == null || clip.getItemCount() == 0) return "";
        CharSequence t = clip.getItemAt(0).coerceToText(this);
        return t == null ? "" : t.toString();
    }

    /** Puts text on the clipboard, for other apps to paste (Rust: clipboard::set). */
    public String setClipboardText(String text) {
        android.content.ClipboardManager cm = (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        if (cm != null) cm.setPrimaryClip(android.content.ClipData.newPlainText("text", text));
        return "";
    }

    /** Opens Android's print dialog on a PDF the app wrote (Rust: print::print); deletes it after. */
    public String printPdf(final String path, final String name) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                android.print.PrintManager pm = (android.print.PrintManager) getSystemService(PRINT_SERVICE);
                if (pm == null) {
                    new java.io.File(path).delete();
                    return;
                }
                try {
                pm.print(name, new android.print.PrintDocumentAdapter() {
                    @Override
                    public void onLayout(android.print.PrintAttributes oldAttributes, android.print.PrintAttributes newAttributes,
                            android.os.CancellationSignal cancel, LayoutResultCallback callback, android.os.Bundle extras) {
                        if (cancel.isCanceled()) {
                            callback.onLayoutCancelled();
                            return;
                        }
                        callback.onLayoutFinished(new android.print.PrintDocumentInfo.Builder(name)
                                .setContentType(android.print.PrintDocumentInfo.CONTENT_TYPE_DOCUMENT).build(), true);
                    }

                    @Override
                    public void onWrite(android.print.PageRange[] pages, final android.os.ParcelFileDescriptor dest,
                            final android.os.CancellationSignal cancel, final WriteResultCallback callback) {
                        // Off the main thread: a large PDF would freeze the app while it copies.
                        new Thread(new Runnable() {
                            @Override
                            public void run() {
                                write(dest, cancel, callback);
                            }
                        }).start();
                    }

                    private void write(android.os.ParcelFileDescriptor dest, android.os.CancellationSignal cancel, WriteResultCallback callback) {
                        try (InputStream in = new FileInputStream(path); OutputStream out = new FileOutputStream(dest.getFileDescriptor())) {
                            byte[] buf = new byte[65536];
                            int n;
                            while ((n = in.read(buf)) > 0) {
                                if (cancel.isCanceled()) {
                                    callback.onWriteCancelled();
                                    return;
                                }
                                out.write(buf, 0, n);
                            }
                            callback.onWriteFinished(new android.print.PageRange[] { android.print.PageRange.ALL_PAGES });
                        } catch (Exception e) {
                            callback.onWriteFailed(e.toString());
                        }
                    }

                    @Override
                    public void onFinish() {
                        new java.io.File(path).delete();
                    }
                }, null);
                } catch (RuntimeException e) {
                    new java.io.File(path).delete();
                }
            }
        });
        return "";
    }

    public void pickTree() {
        runOnUiThread(() -> {
            Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
            i.addFlags(RW | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            startActivityForResult(i, TREE);
        });
    }

    // Folder browsing for DeckCraft's own Open and Save As pages. Answers start with '+' (the
    // value follows) or '!' (an error message follows).

    /** The top folder of a location. */
    public String rootId(String tree) {
        try {
            return "+" + DocumentsContract.getTreeDocumentId(Uri.parse(tree));
        } catch (Exception e) {
            return "!" + e.getMessage();
        }
    }

    /** A folder's contents: a JSON list of [id, name, is a folder]. */
    public String list(String tree, String folder) {
        Uri t = Uri.parse(tree);
        Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(t, folder);
        String[] cols = {
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
        };
        org.json.JSONArray out = new org.json.JSONArray();
        try (Cursor c = getContentResolver().query(children, cols, null, null, null)) {
            if (c == null) return "!this folder can't be read";
            while (c.moveToNext()) {
                boolean dir = DocumentsContract.Document.MIME_TYPE_DIR.equals(c.getString(2));
                org.json.JSONArray item = new org.json.JSONArray();
                item.put(c.getString(0)).put(c.getString(1)).put(dir);
                out.put(item);
            }
            return "+" + out;
        } catch (Exception e) {
            return "!" + e.getMessage();
        }
    }

    /** The document uri of `doc` inside a location. */
    public String docUri(String tree, String doc) {
        return DocumentsContract.buildDocumentUriUsingTree(Uri.parse(tree), doc).toString();
    }

    /** A new file `name` in `folder`; its document uri. */
    public String create(String tree, String folder, String name) {
        try {
            Uri parent = DocumentsContract.buildDocumentUriUsingTree(Uri.parse(tree), folder);
            Uri made = DocumentsContract.createDocument(getContentResolver(), parent, mimeFor(name), name);
            return made == null ? "!the file couldn't be made" : "+" + made;
        } catch (Exception e) {
            return "!" + e.getMessage();
        }
    }

    // "Open with" from the Files app, or this app as the default for a file type: the document
    // arrives in the intent, at start-up or (the app already running) as a new intent.
    @Override
    protected void onCreate(android.os.Bundle saved) {
        super.onCreate(saved);
        opened(getIntent());
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        opened(intent);
    }

    /** Whether the sender let this app write the document (Open with may give read only). */
    boolean canWrite(Uri uri) {
        return checkCallingOrSelfUriPermission(uri, Intent.FLAG_GRANT_WRITE_URI_PERMISSION) == android.content.pm.PackageManager.PERMISSION_GRANTED;
    }

    void opened(Intent i) {
        if (i == null || i.getData() == null) return;
        String action = i.getAction();
        if (!Intent.ACTION_VIEW.equals(action) && !Intent.ACTION_EDIT.equals(action)) return;
        Uri uri = i.getData();
        try {
            // Keeps access after a restart where the sender allows it, so Save still reaches the file.
            getContentResolver().takePersistableUriPermission(uri, i.getFlags() & RW);
        } catch (Exception ignored) {
        }
        nativePicked(canWrite(uri) ? VIEW : VIEW_READ_ONLY, uri.toString(), displayName(uri));
        // Handled: a recreated activity mustn't open it again.
        i.setAction(Intent.ACTION_MAIN);
        i.setData(null);
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        Uri uri = (result == RESULT_OK && data != null) ? data.getData() : null;
        String name = null;
        if (uri != null) {
            try {
                // Keeps access after a restart, so Save still reaches the file.
                getContentResolver().takePersistableUriPermission(uri, data.getFlags() & RW);
            } catch (Exception ignored) {
            }
            name = treeName(uri);
        }
        nativePicked(request, uri == null ? null : uri.toString(), name);
    }

    String treeName(Uri tree) {
        try {
            return displayName(DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree)));
        } catch (Exception e) {
            return "Folder";
        }
    }

    String displayName(Uri uri) {
        try (Cursor c = getContentResolver().query(uri, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst() && !c.isNull(0)) return c.getString(0);
        } catch (Exception ignored) {
        }
        String last = uri.getLastPathSegment();
        return last == null ? "Book.xlsx" : last.substring(last.lastIndexOf('/') + 1);
    }

    /** The picked document into the working copy at `path`. Empty string = success. */
    public String copyIn(String uri, String path) {
        // Read into a temporary file and replace the working copy only once it is complete, so a
        // failed read never damages the copy already there.
        java.io.File part = new java.io.File(path + ".part");
        try (InputStream in = getContentResolver().openInputStream(Uri.parse(uri))) {
            if (in == null) return "the file couldn't be read";
            try (OutputStream out = new FileOutputStream(part)) {
                pump(in, out);
            }
            if (!part.renameTo(new java.io.File(path))) return "the working copy couldn't be replaced";
            return "";
        } catch (Exception e) {
            part.delete();
            return String.valueOf(e.getMessage());
        }
    }

    /** The working copy at `path` back into the picked document. Empty string = success. */
    public String copyOut(String path, String uri) {
        Uri u = Uri.parse(uri);
        OutputStream out = null;
        try {
            try {
                out = getContentResolver().openOutputStream(u, "wt");
            } catch (Exception e) {
                out = getContentResolver().openOutputStream(u, "w");
            }
            if (out == null) return "the file couldn't be written";
            try (InputStream in = new FileInputStream(path)) {
                pump(in, out);
            }
            out.close();
            return "";
        } catch (Exception e) {
            return String.valueOf(e.getMessage());
        }
    }

    static void pump(InputStream in, OutputStream out) throws java.io.IOException {
        byte[] buf = new byte[65536];
        for (int n; (n = in.read(buf)) > 0; ) out.write(buf, 0, n);
    }
}
