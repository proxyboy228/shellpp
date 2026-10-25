package dev.shellpp.android

import android.app.Activity
import android.app.Application
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.provider.Settings
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONObject
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors

/**
 * The process: the Rust engine on the user's vault (started once), the
 * event thread that hands core and session events to the screens, the
 * one-second tick of the core while the app is in the foreground, and the
 * foreground service that keeps open sessions alive (architecture 3).
 */
class App : Application() {
    /** Why the engine did not start, or null when it runs (or still starts). */
    @Volatile
    var startError: String? = null
        private set

    /** The engine runs. */
    @Volatile
    var ready = false
        private set

    lateinit var platform: Platform
        private set

    private val main = Handler(Looper.getMainLooper())
    private val listeners = ConcurrentHashMap<Long, (JSONObject) -> Unit>()
    private val lastEvents = ConcurrentHashMap<Long, JSONObject>()
    private val worker = Executors.newSingleThreadExecutor()

    /** Syncs run apart from the tick and the activity notes, which must not wait for a network. */
    private val syncWorker = Executors.newSingleThreadExecutor()

    /** Open sessions (not closed yet), for the service's notification. */
    val sessions: MutableSet<Long> = ConcurrentHashMap.newKeySet()

    /** The last view of each open session (title, state), for the Sessions screen. */
    val sessionViews = MutableStateFlow<Map<Long, JSONObject>>(emptyMap())

    /** The saved host of each session opened from one (its row shows a dot). */
    private val sessionHosts = ConcurrentHashMap<Long, String>()

    /** The SFTP sessions among them (Sessions opens them in the SFTP screen). */
    val sftpSessions: MutableSet<Long> = ConcurrentHashMap.newKeySet()

    /** The last event of each transfer (`kind` "transfer", UI spec 10). */
    val transfers = MutableStateFlow<Map<Long, JSONObject>>(emptyMap())

    /** Where each transfer's cache file goes, or comes from (SFTP through the cache, run 14). */
    val transferTargets = ConcurrentHashMap<Long, TransferTarget>()

    /** Uploads waiting for a name conflict's answer. */
    val conflicts = MutableStateFlow<List<Conflict>>(emptyList())

    private val lockFlow = MutableStateFlow(LockStatus.STARTING)

    /** The core's lock state, kept current from its events. */
    val lock: StateFlow<LockStatus> = lockFlow

    private val changesFlow = MutableStateFlow(0L)

    /** Grows with every change of records or vaults: lists read again. */
    val changes: StateFlow<Long> = changesFlow

    /** Unlock preferences of this device (fingerprint unlock, D218). */
    val unlockPrefs by lazy { getSharedPreferences("unlock", Context.MODE_PRIVATE) }

    private var started = 0
    private var lastActivityNote = 0L

    override fun onCreate() {
        super.onCreate()
        platform = Platform(this, DeviceKeys(this))
        registerActivityLifecycleCallbacks(Foreground())
        // Transfer files of an earlier process: nothing waits for them any more.
        File(cacheDir, "transfers").deleteRecursively()
        Core.onLocalChange = ::kickSync
        // A network that comes back: a sync (the core keeps its backoff).
        getSystemService(android.net.ConnectivityManager::class.java)?.registerDefaultNetworkCallback(
            object : android.net.ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: android.net.Network) {
                    if (started > 0) syncSoon()
                }
            },
        )
        // The core opens its database off the main thread (architecture 2).
        Thread({
            try {
                Native.start(filesDir.absolutePath, deviceName(), platform)
                // Before any sync: Android's trust manager behind the TLS checks.
                // Without it only sync answers "unavailable" (architecture 5).
                try {
                    Native.initTls(this)
                } catch (error: RuntimeException) {
                    android.util.Log.w("Shell++", "TLS setup failed")
                }
                ready = true
                refreshLock()
                pumpEvents()
            } catch (error: RuntimeException) {
                startError = error.message ?: error.toString()
                lockFlow.value = LockStatus("failed", false, false, false)
            }
        }, "engine").apply { isDaemon = true }.start()
    }

    private fun deviceName(): String {
        val named = try {
            Settings.Global.getString(contentResolver, Settings.Global.DEVICE_NAME)
        } catch (error: SecurityException) {
            null
        }
        return named?.takeIf { it.isNotBlank() } ?: Build.MODEL ?: "Android"
    }

    /** Reads the lock state again (off the main thread). */
    fun refreshLock() {
        if (!ready) return
        try {
            publishLock(Core.lockStatus())
        } catch (error: Exception) {
            // The next event brings it.
        }
    }

    /** Sets the lock state from a command's answer. */
    fun setLock(status: LockStatus) = publishLock(status)

    private val lockListeners = java.util.concurrent.CopyOnWriteArrayList<(LockStatus) -> Unit>()

    /** [listener] hears every lock state on the main thread (a terminal leaves when locked). */
    fun onLockChange(listener: (LockStatus) -> Unit) {
        lockListeners += listener
    }

    fun removeLockListener(listener: (LockStatus) -> Unit) {
        lockListeners -= listener
    }

    /**
     * Every new lock state goes through here, from a command's answer or an
     * event: the change from locked to unlocked starts a sync
     * (architecture 3), whichever path saw it first.
     */
    private fun publishLock(status: LockStatus) {
        val was = synchronized(lockFlow) {
            val previous = lockFlow.value
            lockFlow.value = status
            previous
        }
        if (!was.unlocked && status.unlocked) syncSoon()
        // The first run ended (a new vault, or Restore from sync, whose
        // wizard is gone by then): the phone's own defaults (UI spec 5).
        if (was.firstRun && status.unlocked) afterRestore()
        if (was.state != status.state) main.post { for (listener in lockListeners) listener(status) }
    }

    private fun pumpEvents() {
        while (true) {
            val text = try {
                Native.nextEvent(1000)
            } catch (error: Exception) {
                null
            } ?: continue
            val event = try {
                JSONObject(text)
            } catch (error: org.json.JSONException) {
                continue
            }
            if (event.optString("kind") == "core") {
                onCoreEvent(event)
                continue
            }
            if (event.optString("kind") == "transfer") {
                onTransfer(event)
                continue
            }
            val id = event.getLong("id")
            sessionHosts[id]?.let { event.put("recordId", it) }
            lastEvents[id] = event
            main.post {
                if (event.optString("state") == "closed") {
                    forget(id)
                } else {
                    sessionViews.value = sessionViews.value + (id to event)
                }
                listeners[id]?.invoke(event)
            }
        }
    }

    private fun onCoreEvent(event: JSONObject) {
        when (event.optString("name")) {
            "lock:changed" -> refreshLock()
            "records:changed", "vaults:changed", "settings:changed" -> changesFlow.value = changesFlow.value + 1
        }
    }

    /** Session [id]'s events go to [listener] on the main thread; the last one first. */
    fun listen(id: Long, listener: (JSONObject) -> Unit) {
        listeners[id] = listener
        lastEvents[id]?.let { last -> main.post { listener(last) } }
    }

    fun unlisten(id: Long) {
        listeners.remove(id)
    }

    fun opened(id: Long, title: String, recordId: String?, sftp: Boolean = false) {
        sessions += id
        if (sftp) sftpSessions += id
        if (recordId != null) sessionHosts[id] = recordId
        // An event may have come first: it is kept, with the host added.
        val view = sessionViews.value[id] ?: JSONObject().put("id", id).put("title", title).put("state", "connecting")
        if (recordId != null) view.put("recordId", recordId)
        sessionViews.value = sessionViews.value + (id to view)
        updateService()
        startTicking()
    }

    fun forget(id: Long) {
        sessions -= id
        sftpSessions -= id
        sessionHosts.remove(id)
        // Its transfers are gone with it (the core cancelled them).
        conflicts.value = conflicts.value.filter { conflict ->
            (conflict.session != id).also { keep -> if (!keep) conflict.cache.delete() }
        }
        lastEvents.remove(id)
        sessionViews.value = sessionViews.value - id
        updateService()
    }

    fun closeAll() {
        for (id in sessions.toList()) {
            Native.close(id)
            forget(id)
            listeners[id]?.invoke(JSONObject().put("kind", "session").put("id", id).put("state", "closed"))
        }
    }

    /** The service runs while a session is open (dropped and reconnecting included). */
    fun updateService() {
        // Started again for every change, so the notification counts right;
        // with nothing open the service stops itself once in the foreground.
        val intent = Intent(this, SessionService::class.java)
        if (Build.VERSION.SDK_INT >= 26) startForegroundService(intent)
    }

    /** A touch or a key in the app: the auto-lock counts from the last input (UI spec 5). */
    fun noteActivity() {
        val now = System.currentTimeMillis()
        if (!ready || now - lastActivityNote < 2_000) return
        lastActivityNote = now
        worker.execute {
            try {
                Core.call("activity")
            } catch (error: Exception) {
                // Locked meanwhile: nothing to note.
            }
        }
    }

    /**
     * A sync when the vault is unlocked and bound (after unlock and when the
     * app comes back to the foreground; no timer in the background,
     * architecture 3). Off the main thread; failures wait for the next one.
     */
    fun syncSoon() {
        if (!ready) return
        syncWorker.execute {
            try {
                if (!Core.lockStatus().unlocked) return@execute
                val status = Core.obj("sync_status")
                if (status.optBoolean("bound") && status.optBoolean("signed_in")) Core.call("sync_auto")
            } catch (error: Exception) {
                // No network or a refusal: Account and sync shows the last error.
            }
        }
    }

    private val kick = Runnable { syncSoon() }

    /** One sync 2 s after the last local change, as on the desktop (architecture 3). */
    fun kickSync() {
        main.removeCallbacks(kick)
        main.postDelayed(kick, 2_000)
    }

    /**
     * A transfer's event (UI spec 10). Its end is handled here, whatever
     * screen shows (review of run 14): a finished download is copied into
     * its document, and only then shown as done; a failed copy shows as a
     * failure; a cancelled or failed download leaves no empty document;
     * cache files go once nothing needs them.
     */
    private fun onTransfer(event: JSONObject) {
        val transfer = event.optLong("id")
        val state = event.optString("state")
        val target = transferTargets[transfer]
        val publish = { update: JSONObject ->
            main.post {
                // A cancel's event carries only its state.
                val merged = transfers.value[transfer]?.let { JSONObject(it.toString()) } ?: JSONObject()
                for (key in update.keys()) merged.put(key, update.get(key))
                transfers.value = transfers.value + (transfer to merged)
                transferNotification(transfer, merged.optString("state"))
            }
        }
        if (target == null || state !in setOf("done", "failed", "cancelled")) {
            publish(event)
            return
        }
        transferWorker.execute {
            when (state) {
                "done" -> {
                    transferTargets.remove(transfer)
                    if (target.download && target.uri != null) {
                        val written = try {
                            contentResolver.openOutputStream(target.uri, "wt")?.use { out ->
                                target.cache.inputStream().use { it.copyTo(out) }
                                true
                            } ?: false
                        } catch (error: Exception) {
                            false
                        }
                        if (!written) {
                            deleteDocument(target)
                            event.put("state", "failed").put("error", "The file could not be written where you chose.")
                        }
                    }
                    target.dropCache()
                }
                "cancelled" -> {
                    transferTargets.remove(transfer)
                    deleteDocument(target)
                    target.dropCache()
                }
                // A failure keeps its cache file for Retry (an upload needs it);
                // a download's empty document goes.
                "failed" -> deleteDocument(target)
            }
            publish(event)
        }
    }

    private val transferWorker = Executors.newSingleThreadExecutor()

    private val notifiedStates = HashMap<Long, String>()
    private var notifiedAt = 0L

    /**
     * The service's notification follows the transfers (UI spec 10): at once
     * when one changes state, at most once a second for its bytes. Updated
     * in place, not by starting the service again; on the main thread, so it
     * never comes after the last session's close stopped the service.
     */
    private fun transferNotification(transfer: Long, state: String) {
        val changed = notifiedStates.put(transfer, state) != state
        if (state !in setOf("queued", "running")) notifiedStates.remove(transfer)
        val now = SystemClock.elapsedRealtime()
        if (!changed && now - notifiedAt < 1_000) return
        notifiedAt = now
        if (sessions.isEmpty()) return
        try {
            getSystemService(NotificationManager::class.java)
                .notify(SessionService.NOTIFICATION, SessionService.notification(this, this))
        } catch (error: Exception) {
            // Notifications not allowed: the transfer sheet still shows it.
        }
    }

    private fun deleteDocument(target: TransferTarget) {
        if (!target.download || target.uri == null) return
        try {
            android.provider.DocumentsContract.deleteDocument(contentResolver, target.uri)
        } catch (error: Exception) {
            // The provider does not delete: the empty document stays.
        }
    }

    /** After the first run (a new vault, or Restore from sync): the phone's own defaults (UI spec 5). */
    fun afterRestore() {
        worker.execute {
            try {
                Core.call(
                    "settings_set",
                    JSONObject().put("values", JSONObject().put("security.auto_lock_minutes", 15)),
                )
            } catch (error: Exception) {
                // The default of the core (never) stays.
            }
            unlockPrefs.edit().putLong(Fingerprint.PREF_LAST_PASSWORD, System.currentTimeMillis()).apply()
            refreshLock()
        }
    }

    private fun tick() {
        if (!ready) return
        worker.execute {
            try {
                Core.call("tick")
            } catch (error: Exception) {
                // The next tick tries again.
            }
        }
    }

    @Volatile
    private var ticking = false

    /**
     * The core's tick, every second while a screen is started or a session
     * keeps the process alive (architecture 3): the auto-lock comes on time
     * in the background too.
     */
    private val ticker = object : Runnable {
        override fun run() {
            tick()
            if (started > 0 || sessions.isNotEmpty()) {
                main.postDelayed(this, 1_000)
            } else {
                ticking = false
            }
        }
    }

    private fun startTicking() {
        if (ticking) return
        ticking = true
        main.post(ticker)
    }

    /** Counts the started screens: the core ticks while one is (architecture 3). */
    private inner class Foreground : ActivityLifecycleCallbacks {
        override fun onActivityStarted(activity: Activity) {
            started += 1
            if (started == 1) {
                // A frozen process locks at once when it wakes (the core
                // compares wall-clock time with the last input).
                tick()
                startTicking()
                // Back in the foreground: a sync (architecture 3).
                syncSoon()
            }
        }

        override fun onActivityStopped(activity: Activity) {
            started -= 1
        }

        override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) {}
        override fun onActivityResumed(activity: Activity) {}
        override fun onActivityPaused(activity: Activity) {}
        override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) {}
        override fun onActivityDestroyed(activity: Activity) {}
    }
}

/** A transfer's file in the app's cache and, for a download, the document it goes to. */
data class TransferTarget(val cache: java.io.File, val uri: android.net.Uri?, val download: Boolean) {
    /** The cache file and the `.part` the SFTP code writes beside it. */
    fun dropCache() {
        cache.delete()
        java.io.File(cache.path + ".part").delete()
    }
}

/** An upload whose name exists on the server, waiting for Replace or Skip. */
data class Conflict(val session: Long, val cache: java.io.File, val remote: String, val name: String)
