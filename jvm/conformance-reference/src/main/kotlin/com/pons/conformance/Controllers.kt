package com.pons.conformance

import com.pons.spring.PonsAuth
import com.pons.spring.PonsErrors
import com.pons.spring.PonsOperation
import com.pons.spring.Provides
import org.springframework.http.HttpStatus
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

@RestController
@RequestMapping("/auth")
class AuthController {
    @PostMapping("/login")
    @PonsOperation(
        name = "auth.login",
        summary = "Exchange credentials for a session token.",
        provides = Provides(scheme = "session", from = "body:/token"),
    )
    @PonsErrors(InvalidCredentials::class)
    fun login(@RequestBody request: LoginRequest): Session =
        Session(token = "token", expiresAt = "2026-01-01T00:00:00Z")
}

@RestController
@RequestMapping("/users")
class UserController {
    @GetMapping("/{id}")
    @PonsOperation(name = "users.get", summary = "Read a user.")
    @PonsAuth(scheme = "session")
    fun get(
        @PathVariable id: Long,
        @RequestParam(required = false) active: Boolean,
        @RequestParam(required = false) role: RoleFilter,
    ): User = User(id = id, name = "Ada")
}

@RestController
@RequestMapping("/notifications")
class NotificationController {
    @GetMapping("/latest")
    @PonsOperation(name = "notifications.latest", summary = "Read the latest notification.")
    fun latest(): Notification = Notification.Follow(userId = 1)
}

@RestControllerAdvice
class ApiErrors {
    @ExceptionHandler(AccountLockedException::class)
    @ResponseStatus(HttpStatus.LOCKED)
    fun locked(exception: AccountLockedException): AccountLocked = AccountLocked(retryAfterSecs = 30)
}
