package dev.shellpp.android

import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONObject
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

    /** Open sessions (not closed yet), for the service's notification. */
    val sessions: MutableSet<Long> = ConcurrentHashMap.newKeySet()

    /** The last view of each open session (title, state), for the Sessions screen. */
    val sessionViews = MutableStateFlow<Map<Long, JSONObject>>(emptyMap())

    /** The saved host of each session opened from one (its row shows a dot). */
    private val sessionHosts = ConcurrentHashMap<Long, String>()

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
        // The core opens its database off the main thread (architecture 2).
        Thread({
            try {
                Native.start(filesDir.absolutePath, deviceName(), platform)
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
            lockFlow.value = Core.lockStatus()
        } catch (error: RuntimeException) {
            // The next event brings it.
        }
    }

    /** Sets the lock state from a command's answer. */
    fun setLock(status: LockStatus) {
        lockFlow.value = status
    }

    private fun pumpEvents() {
        while (true) {
            val text = try {
                Native.nextEvent(1000)
            } catch (error: RuntimeException) {
                null
            } ?: continue
            val event = JSONObject(text)
            if (event.optString("kind") == "core") {
                onCoreEvent(event)
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

    fun opened(id: Long, title: String, recordId: String?) {
        sessions += id
        if (recordId != null) sessionHosts[id] = recordId
        val view = JSONObject().put("id", id).put("title", title).put("state", "connecting")
        if (recordId != null) view.put("recordId", recordId)
        sessionViews.value = sessionViews.value + (id to view)
        updateService()
    }

    fun forget(id: Long) {
        sessions -= id
        sessionHosts.remove(id)
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
            } catch (error: RuntimeException) {
                // Locked meanwhile: nothing to note.
            }
        }
    }

    private fun tick() {
        if (!ready) return
        worker.execute {
            try {
                Core.call("tick")
            } catch (error: RuntimeException) {
                // The next tick tries again.
            }
        }
    }

    private val ticker = object : Runnable {
        override fun run() {
            tick()
            if (started > 0) main.postDelayed(this, 1_000)
        }
    }

    /** Counts the started screens: the core ticks while one is (architecture 3). */
    private inner class Foreground : ActivityLifecycleCallbacks {
        override fun onActivityStarted(activity: Activity) {
            started += 1
            if (started == 1) {
                // A frozen process locks at once when it wakes (the core
                // compares wall-clock time with the last input).
                main.removeCallbacks(ticker)
                main.post(ticker)
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
