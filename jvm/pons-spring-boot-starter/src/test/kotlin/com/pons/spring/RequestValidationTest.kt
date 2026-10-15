package com.pons.spring

import com.fasterxml.jackson.databind.ObjectMapper
import com.pons.ir.Info
import com.pons.spring.testapp.RequestRejectionApplication
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.BeforeEach
import org.junit.jupiter.api.Test
import org.springframework.http.MediaType
import org.springframework.mock.web.MockServletContext
import org.springframework.test.web.servlet.MockMvc
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.get
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.head
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.post
import org.springframework.test.web.servlet.result.MockMvcResultMatchers.content
import org.springframework.test.web.servlet.result.MockMvcResultMatchers.jsonPath
import org.springframework.test.web.servlet.result.MockMvcResultMatchers.status
import org.springframework.test.web.servlet.setup.DefaultMockMvcBuilder
import org.springframework.test.web.servlet.setup.MockMvcBuilders
import org.springframework.web.context.support.AnnotationConfigWebApplicationContext

class RequestValidationTest {
    private lateinit var context: AnnotationConfigWebApplicationContext
    private lateinit var mockMvc: MockMvc

    @BeforeEach
    fun setUp() {
        context = AnnotationConfigWebApplicationContext()
        context.servletContext = MockServletContext()
        context.register(RequestRejectionApplication::class.java)
        context.refresh()

        val mapper = context.getBean(ObjectMapper::class.java)
        val exporter = PonsExporter(context, mapper, Info(title = "Request Validation", version = "1"))
        val base = MockMvcBuilders.webAppContextSetup(context)
        val builder: DefaultMockMvcBuilder =
            base.addFilters(PonsRequestValidationFilter(exporter, mapper))
        mockMvc = builder.build()
    }

    @AfterEach
    fun close() {
        context.close()
    }

    @Test
    fun anInvalidPathParameterIsRejectedWithTheDeclaredReport() {
        mockMvc.perform(
            post("/items/abc")
                .queryParam("color", "RED")
                .header("X-Trace", "trace")
                .contentType(MediaType.APPLICATION_JSON)
                .content("""{"name":"item"}"""),
        )
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(content().contentType(MediaType.APPLICATION_JSON))
            .andExpect(jsonPath("$.violations[0].path").value("/id"))
            .andExpect(jsonPath("$.violations[0].reason").value("expected a number, found a string"))
    }

    @Test
    fun anInvalidQueryParameterIsRejectedWithTheDeclaredReport() {
        mockMvc.perform(
            post("/items/1")
                .queryParam("color", "GREEN")
                .header("X-Trace", "trace")
                .contentType(MediaType.APPLICATION_JSON)
                .content("""{"name":"item"}"""),
        )
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(jsonPath("$.violations[0].path").value("/color"))
            .andExpect(jsonPath("$.violations[0].reason").value("unknown enum variant GREEN"))
    }

    @Test
    fun aMissingRequestHeaderIsRejectedWithTheDeclaredReport() {
        mockMvc.perform(
            post("/items/1")
                .queryParam("color", "RED")
                .contentType(MediaType.APPLICATION_JSON)
                .content("""{"name":"item"}"""),
        )
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(jsonPath("$.violations[0].path").value("/X-Trace"))
            .andExpect(jsonPath("$.violations[0].reason").value("missing required field X-Trace"))
    }

    @Test
    fun anInvalidJsonBodyIsRejectedWithTheDeclaredReport() {
        mockMvc.perform(
            post("/items/1")
                .queryParam("color", "RED")
                .header("X-Trace", "trace")
                .contentType(MediaType.APPLICATION_JSON)
                .content("""{"name":12}"""),
        )
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(jsonPath("$.violations[0].path").value("/name"))
            .andExpect(jsonPath("$.violations[0].reason").value("expected a string, found a number"))
    }

    @Test
    fun anUnknownJsonPropertyIsRejectedWithTheDeclaredReport() {
        mockMvc.perform(
            post("/items/1")
                .queryParam("color", "RED")
                .header("X-Trace", "trace")
                .contentType(MediaType.APPLICATION_JSON)
                .content("""{"name":"item","extra":true}"""),
        )
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(jsonPath("$.violations[0].path").value("/extra"))
            .andExpect(jsonPath("$.violations[0].reason").value("unknown property extra"))
    }

    @Test
    fun aValidRequestReachesTheHandlerWithItsBody() {
        mockMvc.perform(
            post("/items/7")
                .queryParam("color", "RED")
                .header("X-Trace", "trace")
                .contentType(MediaType.APPLICATION_JSON)
                .content("""{"name":"item"}"""),
        )
            .andExpect(status().isOk)
            .andExpect(jsonPath("$.token").value("7:RED:trace:item"))
    }

    @Test
    fun aHeadRejectionCarriesNoBody() {
        mockMvc.perform(head("/head").queryParam("id", "abc"))
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(content().bytes(ByteArray(0)))
    }

    @Test
    fun aValidHeadRequestReachesTheHandler() {
        mockMvc.perform(head("/head").queryParam("id", "1"))
            .andExpect(status().isOk)
            .andExpect(content().bytes(ByteArray(0)))
    }

    @Test
    fun aWrongContentTypeIsRejectedWithTheDeclaredReport() {
        mockMvc.perform(
            post("/items/1")
                .queryParam("color", "RED")
                .header("X-Trace", "trace")
                .contentType(MediaType.TEXT_PLAIN)
                .content("""{"name":"item"}"""),
        )
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(content().contentType(MediaType.APPLICATION_JSON))
            .andExpect(jsonPath("$.violations[0].reason").value("expected application/json, found text/plain"))
    }

    @Test
    fun aMissingContentTypeIsRejectedWhenTheBodyIsPresent() {
        mockMvc.perform(
            post("/items/1")
                .queryParam("color", "RED")
                .header("X-Trace", "trace")
                .content("""{"name":"item"}"""),
        )
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(jsonPath("$.violations[0].reason").value("expected application/json, found no content type"))
    }

    @Test
    fun ordinaryRoutesAreUntouched() {
        mockMvc.perform(get("/plain"))
            .andExpect(status().isOk)
            .andExpect(jsonPath("$.token").value("plain"))

        mockMvc.perform(
            post("/framework/note")
                .contentType(MediaType.TEXT_PLAIN)
                .content("note"),
        )
            .andExpect(status().isOk)
            .andExpect(jsonPath("$.token").value("note"))
    }

    @Test
    fun anOperationThatDeclaresItsOwnRejectionIsLeftToItsHandler() {
        mockMvc.perform(
            post("/declared")
                .contentType(MediaType.APPLICATION_JSON)
                .content("""{"name":"item"}"""),
        )
            .andExpect(status().isOk)
            .andExpect(jsonPath("$.token").value("item"))
    }
}
