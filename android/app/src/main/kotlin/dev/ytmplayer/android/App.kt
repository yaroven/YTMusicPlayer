package dev.ytmplayer.android

import android.app.Application
import dev.ytmplayer.core.Auth
import dev.ytmplayer.core.Catalog
import dev.ytmplayer.core.LyricsFinder
import dev.ytmplayer.core.Resolver

/**
 * The app's one copy of each service. The player lives here, not in the
 * activity, so it keeps playing with the screen gone; `PlaybackService`
 * only puts its media session in front of the system.
 */
class App : Application() {
    lateinit var network: Network
        private set
    lateinit var auth: Auth
        private set
    lateinit var catalog: Catalog
        private set
    lateinit var resolver: Resolver
        private set
    lateinit var lyrics: LyricsFinder
        private set
    lateinit var library: Library
        private set
    lateinit var player: Player
        private set
    lateinit var account: Account
        private set
    lateinit var settings: Settings
        private set
    lateinit var updater: Updater
        private set

    override fun onCreate() {
        super.onCreate()
        instance = this
        Covers.init(this)
        settings = Settings(this)
        network = Network(this)
        val store = SecureStore(this)
        // The Google account's youtube.com cookies, encrypted with a
        // Keystore key; rotated cookies are saved back as YouTube sends them.
        auth = Auth(store.cookies()) { store.save(it) }
        catalog = Catalog(auth)
        resolver = Resolver(auth, WebViewJs(this))
        lyrics = LyricsFinder(catalog)
        library = Library(this, catalog, resolver)
        account = Account(store, auth, catalog, resolver, library)
        player = Player(this, library, resolver, catalog, network, settings)
        updater = Updater(this)
        updater.checkDaily()
    }

    companion object {
        lateinit var instance: App
            private set
    }
}
