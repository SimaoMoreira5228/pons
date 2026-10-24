package com.pons.spring.testapp

import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.databind.SerializationFeature
import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper
import com.pons.spring.PonsError
import com.pons.spring.PonsErrors
import org.springframework.context.annotation.Bean
import org.springframework.context.annotation.Configuration
import org.springframework.http.MediaType
import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.PathVariable
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.RequestBody
import org.springframework.web.bind.annotation.RequestHeader
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RequestMethod
import org.springframework.web.bind.annotation.RequestParam
import org.springframework.web.bind.annotation.RestController
import org.springframework.web.servlet.config.annotation.EnableWebMvc
import java.time.Instant

@Configuration(proxyBeanMethods = false)
@EnableWebMvc
class RequestRejectionApplication {
    @Bean
    fun objectMapper(): ObjectMapper = jacksonObjectMapper()
        .findAndRegisterModules()
        .disable(SerializationFeature.WRITE_DATES_AS_TIMESTAMPS)

    @Bean
    fun combinedController(): CombinedController = CombinedController()

    @Bean
    fun plainController(): PlainController = PlainController()

    @Bean
    fun headController(): HeadController = HeadController()

    @Bean
    fun declaredController(): DeclaredController = DeclaredController()

    @Bean
    fun frameworkBodyController(): FrameworkBodyController = FrameworkBodyController()
}

data class ItemRequest(
    val name: String,
    val quantity: Int = 1,
)

enum class ItemColor {
    RED,
    BLUE,
}

@RestController
@RequestMapping("/items")
class CombinedController {
    @PostMapping(
        "/{id}",
        consumes = [MediaType.APPLICATION_JSON_VALUE],
        produces = [MediaType.APPLICATION_JSON_VALUE],
    )
    fun combined(
        @PathVariable id: Long,
        @RequestParam color: ItemColor,
        @RequestHeader("X-Trace") trace: String,
        @RequestBody request: ItemRequest,
    ): Session = Session(token = "$id:$color:$trace:${request.name}", expiresAt = Instant.EPOCH)
}

@RestController
@RequestMapping("/plain")
class PlainController {
    @GetMapping(produces = [MediaType.APPLICATION_JSON_VALUE])
    fun plain(): Session = Session(token = "plain", expiresAt = Instant.EPOCH)
}

@RestController
@RequestMapping("/head")
class HeadController {
    @RequestMapping(method = [RequestMethod.HEAD])
    fun head(@RequestParam id: Long) {
        check(id >= 0)
    }
}

@PonsError(status = 422)
data class AlreadyRejected(val message: String)

@RestController
@RequestMapping("/declared")
class DeclaredController {
    @PostMapping(
        consumes = [MediaType.APPLICATION_JSON_VALUE],
        produces = [MediaType.APPLICATION_JSON_VALUE],
    )
    @PonsErrors(AlreadyRejected::class)
    fun declared(@RequestBody request: ItemRequest): Session =
        Session(token = request.name, expiresAt = Instant.EPOCH)
}

@RestController
@RequestMapping("/framework")
class FrameworkBodyController {
    @PostMapping(
        "/note",
        consumes = [MediaType.TEXT_PLAIN_VALUE],
        produces = [MediaType.APPLICATION_JSON_VALUE],
    )
    fun note(@RequestBody note: String): Session =
        Session(token = note, expiresAt = Instant.EPOCH)
}
