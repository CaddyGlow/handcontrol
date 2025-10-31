package com.handcontrol.data.settings

data class AppSettings(
    val theme: Theme = Theme.SYSTEM,
    val enableRelayFallback: Boolean = true,
    val ipv6Preference: IpPreference = IpPreference.IPV6_PREFERRED,
    val directConnectionTimeoutSeconds: Int = 5,
    val autoDiscovery: Boolean = true,
    val discoveryTimeoutSeconds: Int = 5,
    val logLevel: LogLevel = LogLevel.INFO,
    val enableNetworkLogging: Boolean = false,
    val enableNetworkDiagnostics: Boolean = false
)

enum class Theme {
    LIGHT,
    DARK,
    SYSTEM
}

enum class IpPreference {
    IPV6_PREFERRED,
    IPV4_PREFERRED,
    IPV6_ONLY,
    IPV4_ONLY
}

enum class LogLevel {
    VERBOSE,
    DEBUG,
    INFO,
    WARN,
    ERROR
}

// Extension functions for display strings
fun Theme.toDisplayString(): String = when (this) {
    Theme.LIGHT -> "Light"
    Theme.DARK -> "Dark"
    Theme.SYSTEM -> "System Default"
}

fun IpPreference.toDisplayString(): String = when (this) {
    IpPreference.IPV6_PREFERRED -> "IPv6 Preferred"
    IpPreference.IPV4_PREFERRED -> "IPv4 Preferred"
    IpPreference.IPV6_ONLY -> "IPv6 Only"
    IpPreference.IPV4_ONLY -> "IPv4 Only"
}
