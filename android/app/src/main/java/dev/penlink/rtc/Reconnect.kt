package dev.penlink.rtc

/** State of the WebSocket link to the host, as the UI sees it. */
sealed interface LinkState {
    /** Nothing opened yet, or the connection was shut down on purpose. */
    data object Idle : LinkState

    /** The WebSocket is up. */
    data object Connected : LinkState

    /** The link dropped (or never came up); trying again. [attempt] counts failures in this outage. */
    data class Reconnecting(val attempt: Int) : LinkState

    /** Still no host after [ReconnectMachine.unreachableAfterMs]: the UI offers a Retry button. */
    data object Unreachable : LinkState
}

/**
 * Pure reconnect policy: no threads, no clock of its own (callers pass `now`), so it is unit tested.
 *
 *  - [onDropped] when an established link breaks, or [begin] when the first connection is wanted:
 *    starts an outage and says how long to wait before the next try.
 *  - [onAttemptFailed] after a try that failed: next delay, growing from [backoffMs] and capped.
 *    Once an outage lasts longer than [unreachableAfterMs] the state becomes [LinkState.Unreachable]
 *    (tries go on, slowly, so the phone recovers by itself when the PC comes back).
 *  - [onConnected] ends the outage. [retryNow] is the Retry button: restart the outage clock, try at once.
 */
class ReconnectMachine(
    private val backoffMs: List<Long> = listOf(500, 1_000, 2_000, 4_000, 5_000),
    val unreachableAfterMs: Long = 30_000,
    private val unreachableDelayMs: Long = 10_000,
) {
    var state: LinkState = LinkState.Idle
        private set

    private var outageStart = 0L
    private var failures = 0

    /** Start connecting from scratch. Returns the delay before the first try (0). */
    fun begin(now: Long): Long {
        outageStart = now
        failures = 0
        state = LinkState.Reconnecting(0)
        return 0
    }

    /** The established link broke. Returns the delay before the next try. */
    fun onDropped(now: Long): Long {
        outageStart = now
        failures = 0
        state = LinkState.Reconnecting(0)
        return backoffMs.first()
    }

    /** A connection try failed. Returns the delay before the next one. */
    fun onAttemptFailed(now: Long): Long {
        failures++
        if (now - outageStart > unreachableAfterMs) {
            state = LinkState.Unreachable
            return unreachableDelayMs
        }
        state = LinkState.Reconnecting(failures)
        return backoffMs[failures.coerceAtMost(backoffMs.lastIndex)]
    }

    fun onConnected() {
        failures = 0
        state = LinkState.Connected
    }

    /** The Retry button. Returns the delay before the try (0). */
    fun retryNow(now: Long): Long = begin(now)

    fun stop() {
        state = LinkState.Idle
    }
}
