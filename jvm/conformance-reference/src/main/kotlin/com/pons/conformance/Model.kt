package com.pons.conformance

import com.fasterxml.jackson.annotation.JsonProperty
import com.fasterxml.jackson.annotation.JsonSubTypes
import com.fasterxml.jackson.annotation.JsonTypeInfo
import com.pons.spring.PonsError

data class LoginRequest(
    val email: String,
    val password: String,
)

data class Session(
    val token: String,
    val expiresAt: String,
)

@PonsError(status = 401)
data class InvalidCredentials(val message: String)

@PonsError(status = 423)
data class AccountLocked(val retryAfterSecs: Int)

class AccountLockedException : RuntimeException()

data class User(
    val id: Long,
    val name: String,
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
