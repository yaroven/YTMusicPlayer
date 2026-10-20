package dev.yarek.music

import android.content.Context
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import dev.yarek.music.ui.Root
import dev.yarek.music.ui.YtmTheme

class MainActivity : ComponentActivity() {
    override fun attachBaseContext(base: Context) {
        super.attachBaseContext(Lang.wrap(base))
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // Shelves, titles and chips in the app's language.
        App.instance.catalog.language = if (resources.configuration.locales[0].language == "uk") "uk" else "en"
        setContent { YtmTheme { Root() } }
    }
}
