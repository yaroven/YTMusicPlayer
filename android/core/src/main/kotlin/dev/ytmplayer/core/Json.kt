package dev.ytmplayer.core

import org.json.JSONArray
import org.json.JSONObject

// YouTube Music's JSON is a tree of "renderers"; these helpers find them
// anywhere below a node, so wrapper changes don't break parsing. Same as
// YTMKit's JSON.swift.

/** `null` for JSON's null, the value otherwise. */
internal fun Any?.orNull(): Any? = if (this == null || this == JSONObject.NULL) null else this

/** All values under `key`, depth-first, without descending into matches. */
fun findAll(value: Any?, key: String): List<Any> {
    val out = ArrayList<Any>()
    fun walk(v: Any?) {
        when (v) {
            is JSONObject -> for (k in v.keys()) {
                val child = v.opt(k).orNull() ?: continue
                if (k == key) out.add(child) else walk(child)
            }
            is JSONArray -> for (i in 0 until v.length()) walk(v.opt(i))
        }
    }
    walk(value)
    return out
}

fun findFirst(value: Any?, key: String): Any? = findAll(value, key).firstOrNull()

/** `a/b/0/c` style path into nested objects and arrays. */
fun pointer(value: Any?, path: String): Any? {
    var current = value.orNull()
    for (part in path.split('/')) {
        if (part.isEmpty()) continue
        current = when (current) {
            is JSONObject -> current.opt(part).orNull()
            is JSONArray -> part.toIntOrNull()?.takeIf { it in 0 until current.length() }?.let { current.opt(it).orNull() }
            else -> null
        } ?: return null
    }
    return current
}

fun string(value: Any?, path: String): String? = pointer(value, path) as? String

fun JSONObject.obj(key: String): JSONObject? = opt(key) as? JSONObject
fun JSONObject.arr(key: String): JSONArray? = opt(key) as? JSONArray
fun JSONObject.str(key: String): String? = opt(key) as? String

fun JSONArray.objects(): List<JSONObject> = (0 until length()).mapNotNull { opt(it) as? JSONObject }

/** Concatenated `runs[].text` (or `simpleText`). */
fun text(value: Any?): String {
    val obj = value as? JSONObject ?: return ""
    obj.str("simpleText")?.let { return it }
    return runs(obj).mapNotNull { it.str("text") }.joinToString("")
}

fun runs(value: Any?): List<JSONObject> = (value as? JSONObject)?.arr("runs")?.objects() ?: emptyList()

/** Builds a JSONObject from pairs; nested maps and lists become objects and arrays. */
fun json(vararg pairs: Pair<String, Any?>): JSONObject = toJson(mapOf(*pairs)) as JSONObject

fun toJson(value: Any?): Any? = when (value) {
    null -> JSONObject.NULL
    is Map<*, *> -> JSONObject().also { o -> value.forEach { (k, v) -> if (v != null) o.put(k.toString(), toJson(v)) } }
    is List<*> -> JSONArray().also { a -> value.forEach { a.put(toJson(it)) } }
    else -> value
}
