package com.pons.conformance

import com.pons.spring.PonsAuthKind
import com.pons.spring.PonsAuthScheme
import org.springframework.boot.autoconfigure.SpringBootApplication
import org.springframework.boot.autoconfigure.web.servlet.error.ErrorMvcAutoConfiguration
import org.springframework.boot.runApplication

@SpringBootApplication(exclude = [ErrorMvcAutoConfiguration::class])
@PonsAuthScheme(id = "session", kind = PonsAuthKind.BEARER, format = "jwt")
class ConformanceApplication

fun main(args: Array<String>) {
    runApplication<ConformanceApplication>(*args)
}
