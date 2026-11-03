package com.pons.conformance

import com.fasterxml.jackson.annotation.JsonInclude
import com.fasterxml.jackson.annotation.JsonProperty
import com.fasterxml.jackson.annotation.JsonSubTypes
import com.fasterxml.jackson.annotation.JsonTypeInfo
import com.pons.spring.PonsError
import com.pons.spring.PonsErrorHeader

data class LoginRequest(
    val email: String,
    val password: String,
)

@JsonInclude(JsonInclude.Include.NON_NULL)
data class Session(
    val token: String,
    val expiresAt: String,
    val refreshToken: String?,
)

data class CreateUser(
    val email: String,
    val name: String,
)

@PonsError(status = 401)
data class InvalidCredentials(val message: String)

@PonsError(status = 423)
data class AccountLocked(val retryAfterSecs: Int)

@PonsError(
    status = 429,
    headers = [PonsErrorHeader(name = "Retry-After", required = true)],
)
data class RateLimited(val retryAfterSecs: Int)

class AccountLockedException : RuntimeException()

data class User(
    val id: Long,
    val name: String,
    val nickname: String?,
)

data class Report(
    val id: Long,
    val title: String,
)

enum class RoleFilter {
    @JsonProperty("admin")
    ADMIN,

    @JsonProperty("member")
    MEMBER,
}

@JsonTypeInfo(use = JsonTypeInfo.Id.NAME, include = JsonTypeInfo.As.PROPERTY, property = "type")
@JsonSubTypes(
    JsonSubTypes.Type(value = Notification.Mention::class, name = "mention"),
    JsonSubTypes.Type(value = Notification.Follow::class, name = "follow"),
)
sealed class Notification {
    data class Mention(val postId: Long) : Notification()

    data class Follow(val userId: Long) : Notification()
}

@JsonTypeInfo(use = JsonTypeInfo.Id.NAME, include = JsonTypeInfo.As.WRAPPER_OBJECT)
@JsonSubTypes(
    JsonSubTypes.Type(value = Delivery.Email::class, name = "email"),
    JsonSubTypes.Type(value = Delivery.Push::class, name = "push"),
)
sealed class Delivery {
    data class Email(val address: String) : Delivery()

    data class Push(val device: String) : Delivery()
}
