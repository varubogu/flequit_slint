package com.flequit.app;

import android.app.Activity;
import android.content.ComponentCallbacks2;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.util.Log;

/**
 * Bridges the two Android APIs the Rust platform layer cannot reach on its own.
 *
 * <p>The Storage Access Framework and the activity lifecycle both deliver their
 * results to the {@link Activity}, not to whoever asked. This class forwards
 * them to {@code crates/flequit-platform/src/platform/android.rs}, which holds
 * the pending request and the lifecycle observers.
 *
 * <p>It deliberately holds no state of its own: Android may recreate the
 * activity at any time, and a pick that was in flight is then simply reported
 * as cancelled.
 */
public final class FlequitActivity extends android.app.NativeActivity {

    private static final String TAG = "flequit";

    /** Matches {@code LifecycleEvent::from_code} on the Rust side. */
    private static final int LIFECYCLE_SUSPEND = 0;
    private static final int LIFECYCLE_RESUME = 1;
    private static final int LIFECYCLE_LOW_MEMORY = 2;

    static {
        // Same name as the cdylib produced by `cargo build -p flequit-app`.
        System.loadLibrary("flequit_app");
    }

    private static native void nativeOnActivityResult(
            int requestCode, int resultCode, String uri, String displayName);

    private static native void nativeOnLifecycleEvent(int event);

    // --- Storage Access Framework -----------------------------------------

    /**
     * Opens the system document picker. Called from Rust via JNI.
     *
     * @param mimeType MIME type to filter on, or {@code *}{@code /}{@code *}
     * @param requestCode echoed back to {@link #onActivityResult}
     */
    public void pickDocument(String mimeType, int requestCode) {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType(mimeType);
        // Asking for persistable access here is what lets the app re-read the
        // document after a restart; taking the grant happens in onActivityResult.
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        startActivityForResult(intent, requestCode);
    }

    /**
     * Opens the system "save as" picker. Called from Rust via JNI.
     *
     * @param suggestedName file name shown to the user
     * @param requestCode echoed back to {@link #onActivityResult}
     */
    public void createDocument(String suggestedName, int requestCode) {
        Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("application/octet-stream");
        intent.putExtra(Intent.EXTRA_TITLE, suggestedName);
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        startActivityForResult(intent, requestCode);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);

        Uri uri = resultCode == Activity.RESULT_OK && data != null ? data.getData() : null;
        String reference = null;
        String displayName = null;

        if (uri != null) {
            reference = uri.toString();
            displayName = queryDisplayName(uri);
            takePersistablePermission(uri, data.getFlags());
        }

        // Rust resolves the pending request even when the result is a cancel,
        // so this is called unconditionally.
        nativeOnActivityResult(requestCode, resultCode, reference, displayName);
    }

    /** Reads the user-visible file name; SAF URIs do not contain one. */
    private String queryDisplayName(Uri uri) {
        try (Cursor cursor = getContentResolver().query(uri, null, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                int column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (column >= 0) {
                    return cursor.getString(column);
                }
            }
        } catch (Exception error) {
            // The name is cosmetic: Rust falls back to the URI.
            Log.w(TAG, "could not read the document display name", error);
        }
        return null;
    }

    /**
     * Converts the one-shot grant into one that survives a restart.
     *
     * <p>Without this the URI stops working as soon as the process dies, which
     * would silently break any reference the app persisted.
     */
    private void takePersistablePermission(Uri uri, int intentFlags) {
        int flags = intentFlags
                & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
        if (flags == 0) {
            return;
        }
        try {
            getContentResolver().takePersistableUriPermission(uri, flags);
        } catch (SecurityException error) {
            // Some providers refuse; the URI still works for this session.
            Log.w(TAG, "could not persist access to the selected document", error);
        }
    }

    // --- Lifecycle ---------------------------------------------------------

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
    }

    @Override
    protected void onPause() {
        super.onPause();
        nativeOnLifecycleEvent(LIFECYCLE_SUSPEND);
    }

    @Override
    protected void onResume() {
        super.onResume();
        nativeOnLifecycleEvent(LIFECYCLE_RESUME);
    }

    @Override
    public void onTrimMemory(int level) {
        super.onTrimMemory(level);
        // Below RUNNING_LOW the system is only hinting; reacting to every hint
        // would drop caches the app is about to need again.
        if (level >= ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW) {
            nativeOnLifecycleEvent(LIFECYCLE_LOW_MEMORY);
        }
    }
}
