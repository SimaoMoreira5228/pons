package com.pons.spring

import com.pons.ir.AuthRequirement
import com.pons.ir.AuthScheme
import com.pons.ir.Diagnostic
import com.pons.ir.DiagnosticSeverity
import org.springframework.core.annotation.AnnotatedElementUtils
import org.springframework.web.method.HandlerMethod
import java.lang.reflect.AnnotatedElement

private val securityAnnotations = setOf(
    "org.springframework.security.access.prepost.PreAuthorize",
    "org.springframework.security.access.annotation.Secured",
    "jakarta.annotation.security.RolesAllowed",
)

internal class AuthIntrospector(
    private val defaultScheme: String?,
    private val diagnostics: MutableList<Diagnostic>,
) {
    fun requirement(handler: HandlerMethod): AuthRequirement {
        val requirements = mutableListOf<AuthRequirement>()
        AnnotatedElementUtils.findMergedAnnotation(handler.method, PonsAuth::class.java)
            ?.let { requirements += it.toRequirement() }
        AnnotatedElementUtils.findMergedAnnotation(handler.beanType, PonsAuth::class.java)
            ?.let { requirements += it.toRequirement() }
        if (hasSecurityAnnotation(handler.method) || hasSecurityAnnotation(handler.beanType)) {
            val scheme = defaultScheme
            if (scheme.isNullOrEmpty()) {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Warning,
                    code = "SPRING-W009",
                    message = "a security annotation on ${handler} has no configured default scheme, so no auth requirement is recorded",
                )
            } else {
                requirements += AuthRequirement(alternatives = listOf(listOf(scheme)))
            }
        }
        return merge(requirements)
    }

    private fun PonsAuth.toRequirement(): AuthRequirement {
        val groups = alternatives.toList()
        val requirements = groups.map { it.schemes.toList() }.toMutableList()
        if (scheme.isNotEmpty()) requirements += listOf(scheme)
        val scopes = linkedMapOf<String, List<String>>()
        if (this.scopes.isNotEmpty()) {
            for (alternative in requirements) {
                for (name in alternative) scopes[name] = this.scopes.toList()
            }
        }
        for (group in groups) {
            if (group.scopes.isNotEmpty()) {
                for (name in group.schemes) scopes[name] = group.scopes.toList()
            }
        }
        return AuthRequirement(
            alternatives = canonical(requirements),
            scopes = scopes,
            optional = optional,
        )
    }

    private fun merge(requirements: List<AuthRequirement>): AuthRequirement {
        val speaking = requirements.filter { it.saysSomething() }
        if (speaking.isEmpty()) return AuthRequirement()
        val scopes = linkedMapOf<String, MutableSet<String>>()
        for (requirement in speaking) {
            for ((scheme, values) in requirement.scopes) {
                scopes.getOrPut(scheme) { linkedSetOf() } += values
            }
        }
        val alternatives = speaking
            .map { requirement -> requirement.alternatives.map { it.toSortedSet() } }
            .reduce(::combine)
            .map { it.toList() }
        return AuthRequirement(
            alternatives = canonical(alternatives),
            scopes = scopes.mapValues { it.value.toList() },
            optional = speaking.all { it.optional },
        )
    }

    private fun combine(left: List<Set<String>>, right: List<Set<String>>): List<Set<String>> {
        if (right.isEmpty()) return left
        if (left.isEmpty()) return right
        return left.flatMap { one ->
            right.map { other -> (one + other).toSortedSet() }
        }
    }

    private fun AuthRequirement.saysSomething(): Boolean =
        alternatives.isNotEmpty() || scopes.isNotEmpty() || optional

    private fun canonical(alternatives: List<List<String>>): List<List<String>> {
        val sets = alternatives
            .filter { it.isNotEmpty() }
            .map { it.toSortedSet() }
            .distinct()
            .sortedWith(compareBy({ it.size }, { it.joinToString("\u0000") }))
        return sets
            .filterIndexed { index, set ->
                sets.indices.none { other -> other != index && sets[other].containsAll(set) }
            }
            .map { it.toList() }
    }

    private fun hasSecurityAnnotation(element: AnnotatedElement): Boolean =
        element.annotations.any { it.annotationClass.qualifiedName in securityAnnotations }
}

internal fun authScheme(annotation: PonsAuthScheme, diagnostics: MutableList<Diagnostic>): AuthScheme? {
    if (annotation.id.isEmpty()) {
        diagnostics += Diagnostic(
            severity = DiagnosticSeverity.Error,
            code = "SPRING-E012",
            message = "@PonsAuthScheme needs a non-empty id",
        )
        return null
    }
    return when (annotation.kind) {
        PonsAuthKind.BEARER -> AuthScheme.BearerScheme(annotation.format.ifEmpty { null })
        PonsAuthKind.BASIC -> AuthScheme.BasicScheme
        PonsAuthKind.API_KEY -> {
            if (annotation.name.isEmpty()) {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Error,
                    code = "SPRING-E013",
                    message = "the api key scheme ${annotation.id} needs a carrier name",
                )
                null
            } else {
                val location = when (annotation.location) {
                    PonsApiKeyLocation.HEADER -> com.pons.ir.ApiKeyLocation.Header
                    PonsApiKeyLocation.QUERY -> com.pons.ir.ApiKeyLocation.Query
                    PonsApiKeyLocation.COOKIE -> com.pons.ir.ApiKeyLocation.Cookie
                }
                AuthScheme.ApiKeyScheme(location, annotation.name)
            }
        }
        PonsAuthKind.COOKIE_SESSION -> {
            if (annotation.cookie.isEmpty()) {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Error,
                    code = "SPRING-E014",
                    message = "the cookie session scheme ${annotation.id} needs a cookie name",
                )
                null
            } else {
                AuthScheme.CookieSessionScheme(annotation.cookie)
            }
        }
        PonsAuthKind.CUSTOM -> AuthScheme.CustomScheme(
            description = annotation.description.ifEmpty { annotation.id },
            adapter = annotation.adapter.ifEmpty { null },
        )
    }
}
