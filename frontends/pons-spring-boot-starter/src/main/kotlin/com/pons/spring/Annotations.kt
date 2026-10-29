package com.pons.spring

import kotlin.reflect.KClass

@Target(AnnotationTarget.FUNCTION)
@Retention(AnnotationRetention.RUNTIME)
annotation class PonsOperation(
    val name: String = "",
    val summary: String = "",
    val description: String = "",
    val tags: Array<String> = [],
    val provides: Provides = Provides(scheme = ""),
    val pagination: PonsPagination = PonsPagination(),
)

@Retention(AnnotationRetention.RUNTIME)
annotation class PonsPagination(
    val kind: PonsPaginationKind = PonsPaginationKind.CURSOR,
    val field: String = "",
    val items: String = "",
    val next: String = "",
    val pageSize: String = "",
    val pageSizeDefault: Int = 0,
    val pageSizeMaximum: Int = 0,
)

enum class PonsPaginationKind {
    CURSOR,
    OFFSET,
}

@Retention(AnnotationRetention.RUNTIME)
annotation class Provides(
    val scheme: String,
    val from: String = "",
    val expiresAt: String = "",
)

@Target(AnnotationTarget.CLASS, AnnotationTarget.FUNCTION)
@Retention(AnnotationRetention.RUNTIME)
annotation class PonsIgnore

@Target(AnnotationTarget.CLASS)
@Retention(AnnotationRetention.RUNTIME)
annotation class PonsError(
    val status: Int,
    val content: PonsErrorContent = PonsErrorContent.JSON,
    val headers: Array<PonsErrorHeader> = [],
    val retryable: Boolean = true,
)

@Retention(AnnotationRetention.RUNTIME)
annotation class PonsErrorHeader(
    val name: String,
    val required: Boolean = false,
)

@Target(AnnotationTarget.CLASS, AnnotationTarget.FUNCTION)
@Retention(AnnotationRetention.RUNTIME)
annotation class PonsErrors(vararg val value: KClass<*>)

enum class PonsErrorContent {
    JSON,
    PROBLEM_JSON,
    TEXT,
    EMPTY,
}

@Target(AnnotationTarget.CLASS, AnnotationTarget.FUNCTION)
@Retention(AnnotationRetention.RUNTIME)
annotation class PonsAuth(
    val scheme: String = "",
    val scopes: Array<String> = [],
    val alternatives: Array<PonsAuthAlternative> = [],
    val optional: Boolean = false,
)

@Retention(AnnotationRetention.RUNTIME)
annotation class PonsAuthAlternative(
    val schemes: Array<String>,
    val scopes: Array<String> = [],
)

@Target(AnnotationTarget.CLASS)
@Retention(AnnotationRetention.RUNTIME)
@Repeatable
annotation class PonsAuthScheme(
    val id: String,
    val kind: PonsAuthKind,
    val format: String = "",
    val location: PonsApiKeyLocation = PonsApiKeyLocation.HEADER,
    val name: String = "",
    val cookie: String = "",
    val description: String = "",
    val adapter: String = "",
    val flows: Array<PonsOAuthFlow> = [],
)

@Target(AnnotationTarget.CLASS)
@Retention(AnnotationRetention.RUNTIME)
annotation class PonsAuthSchemes(vararg val value: PonsAuthScheme)

@Retention(AnnotationRetention.RUNTIME)
annotation class PonsOAuthFlow(
    val kind: PonsOAuthFlowKind,
    val authorizationUrl: String = "",
    val tokenUrl: String = "",
    val scopes: Array<PonsOAuthScope> = [],
)

@Retention(AnnotationRetention.RUNTIME)
annotation class PonsOAuthScope(
    val name: String,
    val description: String = "",
)

enum class PonsAuthKind {
    BEARER,
    BASIC,
    API_KEY,
    COOKIE_SESSION,
    OAUTH2,
    CUSTOM,
}

enum class PonsApiKeyLocation {
    HEADER,
    QUERY,
    COOKIE,
}

enum class PonsOAuthFlowKind {
    AUTHORIZATION_CODE,
    CLIENT_CREDENTIALS,
    IMPLICIT,
    PASSWORD,
}

@Target(AnnotationTarget.FUNCTION)
@Retention(AnnotationRetention.RUNTIME)
@Repeatable
annotation class PonsResponseHeader(
    val name: String,
    val required: Boolean = false,
    val type: PonsHeaderType = PonsHeaderType.STRING,
)

enum class PonsHeaderType {
    STRING,
    INTEGER,
    BOOLEAN,
    DATE_TIME,
}
