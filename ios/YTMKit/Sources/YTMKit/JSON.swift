import Foundation

/// YouTube Music's JSON is a tree of "renderers"; these helpers find them
/// anywhere below a node, so wrapper changes don't break parsing.
typealias JSON = [String: Any]

/// All values under `key`, depth-first, without descending into matches.
func findAll(_ value: Any, _ key: String) -> [Any] {
    var out: [Any] = []
    func walk(_ v: Any) {
        if let object = v as? JSON {
            for (k, child) in object {
                if k == key { out.append(child) } else { walk(child) }
            }
        } else if let array = v as? [Any] {
            array.forEach(walk)
        }
    }
    walk(value)
    return out
}

func findFirst(_ value: Any, _ key: String) -> Any? {
    findAll(value, key).first
}

/// `a/b/0/c` style path into nested objects and arrays.
func pointer(_ value: Any?, _ path: String) -> Any? {
    var current = value
    for part in path.split(separator: "/") {
        if let object = current as? JSON {
            current = object[String(part)]
        } else if let array = current as? [Any], let i = Int(part), array.indices.contains(i) {
            current = array[i]
        } else {
            return nil
        }
    }
    return current
}

func string(_ value: Any?, _ path: String) -> String? {
    pointer(value, path) as? String
}

/// Concatenated `runs[].text` (or `simpleText`).
func text(_ value: Any?) -> String {
    guard let object = value as? JSON else { return "" }
    if let simple = object["simpleText"] as? String { return simple }
    return runs(object).compactMap { $0["text"] as? String }.joined()
}

func runs(_ value: Any?) -> [JSON] {
    (value as? JSON)?["runs"] as? [JSON] ?? []
}
