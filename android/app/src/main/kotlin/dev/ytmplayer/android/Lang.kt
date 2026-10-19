package dev.ytmplayer.android

import android.app.Activity
import android.app.LocaleManager
import android.content.Context
import android.content.res.Configuration
import android.os.Build
import android.os.LocaleList
import java.util.Locale

/**
 * English or Ukrainian, chosen in the app. Android 13+ keeps the choice
 * per app itself (also in Settings → Apps → Language); older versions get
 * it from our settings, applied to each activity's context.
 */
object Lang {
    fun current(context: Context): String = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        context.getSystemService(LocaleManager::class.java).applicationLocales.get(0)?.language ?: ""
    } else {
        App.instance.settings.language
    }

    fun set(context: Context, tag: String) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            context.getSystemService(LocaleManager::class.java).applicationLocales =
                if (tag.isEmpty()) LocaleList.getEmptyLocaleList() else LocaleList.forLanguageTags(tag)
        } else {
            App.instance.settings.language = tag
            apply(context.applicationContext)
            (context as? Activity)?.recreate()
        }
    }

    fun label(context: Context): String = when (current(context)) {
        "en" -> "English"
        "uk" -> "Українська"
        else -> context.getString(R.string.language_system)
    }

    /** Android 12 and older: the chosen language for a context. */
    fun wrap(base: Context): Context {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) return base
        val tag = base.getSharedPreferences("settings", Context.MODE_PRIVATE).getString("language", "") ?: ""
        if (tag.isEmpty()) return base
        val config = Configuration(base.resources.configuration)
        config.setLocale(Locale.forLanguageTag(tag))
        return base.createConfigurationContext(config)
    }

    /** The application's own resources (status messages come from them). */
    @Suppress("DEPRECATION")
    fun apply(app: Context) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) return
        val tag = App.instance.settings.language
        val locale = if (tag.isEmpty()) LocaleList.getDefault()[0] else Locale.forLanguageTag(tag)
        val config = Configuration(app.resources.configuration)
        config.setLocale(locale)
        app.resources.updateConfiguration(config, app.resources.displayMetrics)
    }
}
