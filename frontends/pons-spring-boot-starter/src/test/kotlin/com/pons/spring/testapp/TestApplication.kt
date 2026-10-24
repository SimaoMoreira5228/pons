package com.pons.spring.testapp

import com.fasterxml.jackson.annotation.JsonFormat
import com.fasterxml.jackson.annotation.JsonIgnore
import com.fasterxml.jackson.annotation.JsonIgnoreProperties
import com.fasterxml.jackson.annotation.JsonInclude
import com.fasterxml.jackson.annotation.JsonProperty
import com.fasterxml.jackson.annotation.JsonSubTypes
import com.fasterxml.jackson.annotation.JsonTypeInfo
import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.databind.SerializationFeature
import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper
import com.pons.spring.PonsAuth
import com.pons.spring.PonsAuthKind
import com.pons.spring.PonsAuthScheme
import com.pons.spring.PonsError
import com.pons.spring.PonsErrorHeader
import com.pons.spring.PonsErrors
import com.pons.spring.PonsHeaderType
import com.pons.spring.PonsOAuthFlow
import com.pons.spring.PonsOAuthFlowKind
import com.pons.spring.PonsOAuthScope
import com.pons.spring.PonsOperation
import com.pons.spring.PonsResponseHeader
import com.pons.spring.Provides
import org.springframework.context.annotation.Bean
import org.springframework.context.annotation.Configuration
import org.springframework.http.HttpStatus
import org.springframework.http.MediaType
import org.springframework.web.bind.annotation.ExceptionHandler
import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.PathVariable
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.RequestBody
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RequestParam
import org.springframework.web.bind.annotation.ResponseStatus
import org.springframework.web.bind.annotation.RestController
import org.springframework.web.bind.annotation.RestControllerAdvice
import org.springframework.web.servlet.config.annotation.EnableWebMvc
import java.math.BigDecimal
import java.time.Instant

@Configuration(proxyBeanMethods = false)
@EnableWebMvc
@PonsAuthScheme(id = "session", kind = PonsAuthKind.BEARER, format = "jwt")
@PonsAuthScheme(
    id = "oauth",
    kind = PonsAuthKind.OAUTH2,
    flows = [
        PonsOAuthFlow(
            kind = PonsOAuthFlowKind.AUTHORIZATION_CODE,
            authorizationUrl = "https://auth.example.test/authorize",
            tokenUrl = "https://auth.example.test/token",
            scopes = [PonsOAuthScope(name = "profile:read", description = "Read the profile")],
        ),
        PonsOAuthFlow(
            kind = PonsOAuthFlowKind.CLIENT_CREDENTIALS,
            tokenUrl = "https://auth.example.test/token",
            scopes = [PonsOAuthScope(name = "profile:read", description = "Read the profile")],
        ),
    ],
)
class TestApplication {
    @Bean
    fun objectMapper(): ObjectMapper = jacksonObjectMapper()
        .disable(SerializationFeature.WRITE_DATES_AS_TIMESTAMPS)

    @Bean
    fun authController(): AuthController = AuthController()

    @Bean
    fun userController(): UserController = UserController()

    @Bean
    fun notificationController(): NotificationController = NotificationController()

    @Bean
    fun apiErrors(): ApiErrors = ApiErrors()
}

data class LoginRequest(
    val email: String,
    val password: String,
    val rememberMe: Boolean = false,
)

@JsonIgnoreProperties(ignoreUnknown = false)
data class Session(
    val token: String,
    val expiresAt: Instant,
)

enum class Role {
    @JsonProperty("admin")
    ADMIN,

    @JsonProperty("user")
    USER,
}

@JsonInclude(JsonInclude.Include.NON_NULL)
data class User(
    @field:JsonFormat(shape = JsonFormat.Shape.STRING)
    val id: Long,
    val email: String,
    val role: Role,
    val balance: BigDecimal,
    val createdAt: Instant,
    val nickname: String? = null,
    @get:JsonIgnore
    val internalNote: String = "",
    val deletedAt: Instant? = null,
)

@JsonTypeInfo(use = JsonTypeInfo.Id.NAME, include = JsonTypeInfo.As.PROPERTY, property = "kind")
@JsonSubTypes(
    JsonSubTypes.Type(value = EmailNotification::class, name = "email"),
    JsonSubTypes.Type(value = SmsNotification::class, name = "sms"),
)
sealed class Notification

data class EmailNotification(val address: String) : Notification()

data class SmsNotification(val number: String) : Notification()

@PonsError(status = 401)
data class InvalidCredentials(val code: String = "invalid_credentials")

@PonsError(
    status = 423,
    headers = [PonsErrorHeader(name = "Retry-After", required = true)],
)
data class AccountLocked(
    val code: String = "locked",
    val retryAfterSecs: Int,
)

class AccountLockedException : RuntimeException()

@RestController
@RequestMapping("/auth")
class AuthController {
    @PostMapping(
        "/login",
        consumes = [MediaType.APPLICATION_JSON_VALUE],
        produces = [MediaType.APPLICATION_JSON_VALUE],
    )
    @PonsOperation(name = "auth.login",
        summary = "Exchange credentials for a session token.",
        provides = Provides(
            scheme = "session",
            from = "body:/token",
            expiresAt = "body:/expiresAt",
        ),
    )
    @PonsErrors(InvalidCredentials::class)
    fun login(@RequestBody request: LoginRequest): Session =
        Session(token = "token", expiresAt = Instant.parse("2026-01-01T00:00:00Z"))
}

@RestController
@RequestMapping("/users")
class UserController {
    @GetMapping("/{id}", produces = [MediaType.APPLICATION_JSON_VALUE])
    @PonsOperation(name = "users.get", summary = "Read a user.")
    @PonsAuth(scheme = "session")
    @PonsResponseHeader(name = "X-Request-Id", required = true)
    @PonsResponseHeader(name = "Cache-Control", type = PonsHeaderType.STRING)
    fun get(
        @PathVariable id: Long,
        @RequestParam(required = false) verbose: Boolean?,
        @RequestParam(required = false) role: Role?,
    ): User = User(
        id = id,
        email = "user@example.test",
        role = Role.ADMIN,
        balance = BigDecimal("12.50"),
        createdAt = Instant.parse("2026-01-01T00:00:00Z"),
        nickname = null,
    )
}

@RestController
@RequestMapping("/notifications")
@PonsAuth(scheme = "oauth", scopes = ["profile:read"])
class NotificationController {
    @GetMapping("/latest", produces = [MediaType.APPLICATION_JSON_VALUE])
    @PonsOperation(name = "notifications.latest", summary = "Read the latest notification.")
    fun latest(): Notification = EmailNotification("user@example.test")
}

@RestControllerAdvice
class ApiErrors {
    @ExceptionHandler(AccountLockedException::class)
    @ResponseStatus(HttpStatus.LOCKED)
    fun locked(exception: AccountLockedException): AccountLocked =
        AccountLocked(retryAfterSecs = 30)
}
