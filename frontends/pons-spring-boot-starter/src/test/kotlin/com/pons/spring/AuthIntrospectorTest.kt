package com.pons.spring

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test
import org.springframework.web.method.HandlerMethod

class AuthIntrospectorTest {
    @PonsAuth(scheme = "session")
    class Controller {
        @PonsAuth(scheme = "api_key")
        fun handler() = Unit
    }

    @Test
    fun classAndMethodRequirementsAreAllOf() {
        val handler = HandlerMethod(
            Controller(),
            Controller::class.java.getMethod("handler"),
        )
        val requirement = AuthIntrospector(null, mutableListOf()).requirement(handler)
        assertEquals(listOf(listOf("api_key", "session")), requirement.alternatives)
    }
}
