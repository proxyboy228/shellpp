package dev.shellpp.android

import android.app.Application
import android.content.Intent
import android.os.Build
import android.os.Handler
import android.os.Looper
import org.json.JSONObject
import java.util.concurrent.ConcurrentHashMap

/**
 * The process: the Rust engine (started once), the event thread that hands
 * session events to their screens, and the foreground service that keeps
 * open sessions alive (architecture 3).
 */
class App : Application() {
    /** Why the engine did not start, or null when it runs. */
    var startError: String? = null
        private set

    private val main = Handler(Looper.getMainLooper())
    private val listeners = ConcurrentHashMap<Long, (JSONObject) -> Unit>()
    private val lastEvents = ConcurrentHashMap<Long, JSONObject>()

    /** Open sessions (not closed yet), for the service's notification. */
    val sessions: MutableSet<Long> = ConcurrentHashMap.newKeySet()

    override fun onCreate() {
        super.onCreate()
        startError = try {
            Native.start(filesDir.absolutePath)
            null
        } catch (error: RuntimeException) {
            error.message ?: error.toString()
        }
        if (startError == null) {
            Thread(::pumpEvents, "session-events").apply { isDaemon = true }.start()
        }
    }

    private fun pumpEvents() {
        while (true) {
            val text = try {
                Native.nextEvent(1000)
            } catch (error: RuntimeException) {
                null
            } ?: continue
            val event = JSONObject(text)
            val id = event.getLong("id")
            lastEvents[id] = event
            main.post {
                if (event.optString("state") == "closed") forget(id)
                listeners[id]?.invoke(event)
            }
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

    fun opened(id: Long) {
        sessions += id
        updateService()
    }

    fun forget(id: Long) {
        sessions -= id
        lastEvents.remove(id)
        updateService()
    }

    fun closeAll() {
        for (id in sessions.toList()) {
            Native.close(id)
            forget(id)
            listeners[id]?.invoke(JSONObject().put("id", id).put("state", "closed"))
        }
    }

    /** The service runs while a session is open (dropped and reconnecting included). */
    fun updateService() {
        val intent = Intent(this, SessionService::class.java)
        if (sessions.isEmpty()) {
            stopService(intent)
        } else if (Build.VERSION.SDK_INT >= 26) {
            startForegroundService(intent)
        }
    }
}
