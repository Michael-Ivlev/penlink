package dev.penlink.notify

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/**
 * Process-wide view of the running notification links, so the settings page can show their state and
 * the "dismissed on the phone" receiver can reach the right socket. Written by the service only.
 */
object NotifyHub {
    private val links = java.util.concurrent.ConcurrentHashMap<String, NotifyLink>()
    private val _status = MutableStateFlow<Map<String, NotifyStatus>>(emptyMap())

    /** PC address -> state of its notification socket (absent = not forwarding). */
    val status: StateFlow<Map<String, NotifyStatus>> = _status.asStateFlow()

    fun register(link: NotifyLink) {
        links[link.host.address] = link
    }

    fun unregister(address: String) {
        links.remove(address)
        _status.update { it - address }
    }

    fun publish(address: String, s: NotifyStatus) {
        _status.update { it + (address to s) }
    }

    fun dismiss(address: String, id: Long): Boolean = links[address]?.dismiss(id) ?: false
}
