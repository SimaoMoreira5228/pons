package com.pons.spring

import com.pons.ir.Diagnostic
import com.pons.ir.DiagnosticSeverity
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test

class OAuthFlowTest {
    @PonsAuthScheme(
        id = "broken",
        kind = PonsAuthKind.OAUTH2,
        flows = [
            PonsOAuthFlow(
                kind = PonsOAuthFlowKind.AUTHORIZATION_CODE,
                tokenUrl = "https://auth.example.test/token",
            ),
        ],
    )
    private class MissingAuthorizationUrl

    @PonsAuthScheme(id = "empty", kind = PonsAuthKind.OAUTH2)
    private class NoFlows

    @Test
    fun rejectsAnIncompleteFlow() {
        val diagnostics = mutableListOf<Diagnostic>()
        val scheme = authScheme(
            MissingAuthorizationUrl::class.java.getAnnotation(PonsAuthScheme::class.java),
            diagnostics,
        )
        assertNull(scheme)
        assertEquals(DiagnosticSeverity.Error, diagnostics.single().severity)
        assertEquals("SPRING-E025", diagnostics.single().code)
        assertTrue(diagnostics.single().message.contains("authorization URL"))
    }

    @Test
    fun rejectsAnOAuth2SchemeWithoutFlows() {
        val diagnostics = mutableListOf<Diagnostic>()
        val scheme = authScheme(
            NoFlows::class.java.getAnnotation(PonsAuthScheme::class.java),
            diagnostics,
        )
        assertNull(scheme)
        assertEquals("SPRING-E024", diagnostics.single().code)
    }
}
