package com.pons.spring

import org.springframework.boot.context.properties.ConfigurationProperties

@ConfigurationProperties("pons")
class PonsProperties {
    var export = Export()
    var endpoint = Endpoint()
    var info = Info()
    var auth = Auth()

    class Export {
        var path: String? = null
        var exit: Boolean = false
    }

    class Endpoint {
        var enabled: Boolean = false
    }

    class Info {
        var title: String? = null
        var version: String = "0.0.0"
        var description: String? = null
        var servers: List<String> = emptyList()
    }

    class Auth {
        var defaultScheme: String? = null
    }
}
