package com.pons.spring

import org.springframework.boot.context.properties.ConfigurationProperties

@ConfigurationProperties("pons")
class PonsProperties {
    var export = Export()
    var endpoint = Endpoint()
    var info = Info()
    var auth = Auth()
    var validation = Validation()

    class Export {
        var path: String? = null
        var exit: Boolean = false
    }

    class Endpoint {
        var enabled: Boolean = false
    }

    class Validation {
        var enabled: Boolean = true
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
