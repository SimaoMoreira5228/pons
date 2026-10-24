package com.pons.spring

import org.springframework.http.MediaType
import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.RestController

@PonsIgnore
@RestController
class PonsIrEndpoint(
    private val exporter: PonsExporter,
) {
    @GetMapping("/_pons/ir", produces = [MediaType.APPLICATION_JSON_VALUE])
    fun document(): String = exporter.render()
}
