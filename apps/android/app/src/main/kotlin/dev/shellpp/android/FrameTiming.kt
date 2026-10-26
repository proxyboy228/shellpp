package dev.shellpp.android

/**
 * How long the terminal takes per frame (plan of run 16, 3.3; D224): the
 * snapshot (the Rust copy of the screen and its parse here) on the pump
 * thread, and the drawing in onDraw. Counted always (two additions per
 * frame); read by SnapshotTimingTest.
 */
object FrameTiming {
    private val lock = Any()
    private var snapshots = 0L
    private var snapshotNanos = 0L
    private var snapshotMax = 0L
    private var cells = 0L
    private var draws = 0L
    private var drawNanos = 0L
    private var drawMax = 0L

    fun snapshot(nanos: Long, cellCount: Int) = synchronized(lock) {
        snapshots += 1
        snapshotNanos += nanos
        snapshotMax = maxOf(snapshotMax, nanos)
        cells += cellCount
    }

    fun drawn(nanos: Long) = synchronized(lock) {
        draws += 1
        drawNanos += nanos
        drawMax = maxOf(drawMax, nanos)
    }

    fun reset() = synchronized(lock) {
        snapshots = 0
        snapshotNanos = 0
        snapshotMax = 0
        cells = 0
        draws = 0
        drawNanos = 0
        drawMax = 0
    }

    /** The counts and times since [reset], in microseconds. */
    fun report(): Map<String, Long> = synchronized(lock) {
        mapOf(
            "snapshots" to snapshots,
            "snapshotAvgUs" to if (snapshots == 0L) 0 else snapshotNanos / snapshots / 1000,
            "snapshotMaxUs" to snapshotMax / 1000,
            "cellsPerSnapshot" to if (snapshots == 0L) 0 else cells / snapshots,
            "draws" to draws,
            "drawAvgUs" to if (draws == 0L) 0 else drawNanos / draws / 1000,
            "drawMaxUs" to drawMax / 1000,
        )
    }
}
