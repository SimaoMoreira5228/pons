@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.pons.ir

import kotlinx.serialization.EncodeDefault
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonClassDiscriminator
import kotlinx.serialization.json.JsonElement

@Serializable
data class AuthRequirement(
    @EncodeDefault(EncodeDefault.Mode.ALWAYS)
    val alternatives: List<List<String>> = emptyList(),
    val scopes: Map<String, List<String>> = emptyMap(),
    @EncodeDefault(EncodeDefault.Mode.ALWAYS)
    val optional: Boolean = false,
    val ext: Map<String, JsonElement> = emptyMap(),
)

@Serializable
@JsonClassDiscriminator("kind")
sealed class AuthScheme {
    @Serializable
    @SerialName("bearer")
    data class BearerScheme(val format: String? = null) : AuthScheme()

    @Serializable
    @SerialName("basic")
    data object BasicScheme : AuthScheme()

    @Serializable
    @SerialName("api_key")
    data class ApiKeyScheme(
        @SerialName("in") val location: ApiKeyLocation,
        val name: String,
    ) : AuthScheme()

    @Serializable
    @SerialName("cookie_session")
    data class CookieSessionScheme(val cookie: String) : AuthScheme()

    @Serializable
    @SerialName("o_auth2")
    data class OAuth2Scheme(val flows: List<OAuthFlow>) : AuthScheme()

    @Serializable
    @SerialName("custom")
    data class CustomScheme(
        val description: String,
        val adapter: String? = null,
        val ext: Map<String, JsonElement> = emptyMap(),
    ) : AuthScheme()
}

@Serializable
enum class ApiKeyLocation {
    @SerialName("header") Header,
    @SerialName("query") Query,
    @SerialName("cookie") Cookie,
}

@Serializable
data class OAuthFlow(
    val kind: OAuthFlowKind,
    @SerialName("authorization_url") val authorizationUrl: String? = null,
    @SerialName("token_url") val tokenUrl: String? = null,
    val scopes: Map<String, String> = emptyMap(),
    val ext: Map<String, JsonElement> = emptyMap(),
)

@Serializable
enum class OAuthFlowKind {
    @SerialName("authorization_code") AuthorizationCode,
    @SerialName("client_credentials") ClientCredentials,
    @SerialName("password") Password,
    @SerialName("implicit") Implicit,
}

@Serializable
data class CredentialProvision(
    val scheme: String,
    val value: CredentialLocator,
    @SerialName("expires_at") val expiresAt: CredentialLocator? = null,
    val ext: Map<String, JsonElement> = emptyMap(),
)

@Serializable
@JsonClassDiscriminator("from")
sealed class CredentialLocator {
    @Serializable
    @SerialName("body")
    data class BodyLocator(val pointer: String) : CredentialLocator()

    @Serializable
    @SerialName("header")
    data class HeaderLocator(val name: String) : CredentialLocator()

    @Serializable
    @SerialName("set_cookie")
    data class SetCookieLocator(val name: String) : CredentialLocator()
}
