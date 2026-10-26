package dev.shellpp.android

import android.content.Context
import android.media.AudioManager
import android.media.ToneGenerator
import android.os.Build
import android.os.SystemClock
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager

/**
 * The terminal's bell on the phone (Settings → Bell, D223 items 5 and 11):
 * Off, Vibrate (a setting of this device) or Sound (the core's
 * `terminal.bell_sound`, as on the desktop). Rings at most once a second;
 * the sound follows the phone's ringer mode (silent: none).
 */
class Bell(private val context: Context) {
    private var last = 0L

    /** On the terminal's pump thread: the settings are read here, not on the main thread. */
    fun ring() {
        val now = SystemClock.elapsedRealtime()
        if (now - last < MIN_INTERVAL_MS) return
        when (mode(context)) {
            Mode.VIBRATE -> {
                last = now
                vibrate()
            }
            Mode.SOUND -> {
                last = now
                sound()
            }
            Mode.OFF -> {}
        }
    }

    private fun vibrate() {
        val vibrator = if (Build.VERSION.SDK_INT >= 31) {
            context.getSystemService(VibratorManager::class.java)?.defaultVibrator
        } else {
            @Suppress("DEPRECATION")
            context.getSystemService(Vibrator::class.java)
        } ?: return
        if (!vibrator.hasVibrator()) return
        vibrator.vibrate(VibrationEffect.createOneShot(VIBRATION_MS, VibrationEffect.DEFAULT_AMPLITUDE))
    }

    private fun sound() {
        val audio = context.getSystemService(AudioManager::class.java)
        if (audio?.ringerMode != AudioManager.RINGER_MODE_NORMAL) return
        runCatching {
            val tone = ToneGenerator(AudioManager.STREAM_NOTIFICATION, 60)
            tone.startTone(ToneGenerator.TONE_PROP_BEEP, TONE_MS)
            // Released after the tone; the generator holds a native player.
            android.os.Handler(android.os.Looper.getMainLooper()).postDelayed({ tone.release() }, TONE_MS + 100L)
        }
    }

    enum class Mode { OFF, VIBRATE, SOUND }

    companion object {
        const val MIN_INTERVAL_MS = 1000L
        const val VIBRATION_MS = 60L
        const val TONE_MS = 120

        /** The bell's setting: the core's sound first, else this device's vibration. */
        fun mode(context: Context): Mode = when {
            coreSettings().optBoolean("terminal.bell_sound", false) -> Mode.SOUND
            TerminalPrefs.of(context).getBoolean(TerminalPrefs.BELL_VIBRATE, false) -> Mode.VIBRATE
            else -> Mode.OFF
        }

        /** Sets the bell's setting (Settings → Bell). */
        fun set(context: Context, mode: Mode) {
            setCoreSetting("terminal.bell_sound", mode == Mode.SOUND)
            TerminalPrefs.of(context).edit().putBoolean(TerminalPrefs.BELL_VIBRATE, mode == Mode.VIBRATE).apply()
        }
    }
}
