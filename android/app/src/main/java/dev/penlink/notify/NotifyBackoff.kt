package dev.penlink.notify

/** Waits between tries to reach a PC's notification socket: quick at first, then every 30 s while the PC is away. */
object NotifyBackoff {
    private val steps = listOf(1_000L, 2_000L, 5_000L, 10_000L, 30_000L)

    /** [failures] = consecutive failed tries so far (0 = the first retry after a drop). */
    fun delayMs(failures: Int): Long = steps[failures.coerceIn(0, steps.size - 1)]
}
