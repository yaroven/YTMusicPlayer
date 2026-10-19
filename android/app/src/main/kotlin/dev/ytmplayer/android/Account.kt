package dev.ytmplayer.android

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.ytmplayer.core.Auth
import dev.ytmplayer.core.Catalog
import dev.ytmplayer.core.Resolver
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/** The signed-in Google account: who it is, sign in / out. */
class Account(
    private val store: SecureStore,
    private val auth: Auth,
    private val catalog: Catalog,
    private val resolver: Resolver,
    private val library: Library,
) {
    var signedIn by mutableStateOf(auth.signedIn)
        private set
    var name by mutableStateOf<String?>(null)
        private set
    var photo by mutableStateOf<String?>(null)
        private set

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    init {
        if (signedIn) loadName()
    }

    fun signIn(cookies: Map<String, String>) {
        store.save(cookies)
        auth.set(cookies)
        signedIn = true
        scope.launch {
            resolver.reset()
            library.sync()
        }
        loadName()
    }

    fun signOut() {
        store.delete()
        auth.set(emptyMap())
        signedIn = false
        name = null
        photo = null
        library.forgetAccount()
        scope.launch { resolver.reset() }
    }

    fun loadName() {
        scope.launch {
            val info = runCatching { catalog.accountInfo() }.getOrNull()
            name = info?.name
            photo = info?.photo
        }
    }
}
