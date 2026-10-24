package com.pons.conformance

import com.pons.spring.PonsApiKeyLocation
import com.pons.spring.PonsAuthKind
import com.pons.spring.PonsAuthScheme
import org.springframework.boot.autoconfigure.SpringBootApplication
import org.springframework.boot.runApplication

@SpringBootApplication
@PonsAuthScheme(id = "session", kind = PonsAuthKind.BEARER, format = "jwt")
@PonsAuthScheme(
    id = "api_key",
    kind = PonsAuthKind.API_KEY,
    location = PonsApiKeyLocation.HEADER,
    name = "X-API-Key",
)
class ConformanceApplication

fun main(args: Array<String>) {
    runApplication<ConformanceApplication>(*args)
}
