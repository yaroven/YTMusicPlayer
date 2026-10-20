import Foundation
import Security

/// youtube.com cookies as one Keychain item (this device only).
enum Keychain {
    private static let query: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: "dev.yarek.music",
        kSecAttrAccount as String: "youtube-cookies",
    ]

    static func cookies() -> [String: String]? {
        var request = query
        request[kSecReturnData as String] = true
        var result: AnyObject?
        guard SecItemCopyMatching(request as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data else { return nil }
        return try? JSONDecoder().decode([String: String].self, from: data)
    }

    static func save(_ cookies: [String: String]) {
        guard let data = try? JSONEncoder().encode(cookies) else { return }
        let update = [kSecValueData as String: data]
        if SecItemUpdate(query as CFDictionary, update as CFDictionary) == errSecItemNotFound {
            var item = query
            item[kSecValueData as String] = data
            item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            SecItemAdd(item as CFDictionary, nil)
        }
    }

    static func delete() {
        SecItemDelete(query as CFDictionary)
    }
}
