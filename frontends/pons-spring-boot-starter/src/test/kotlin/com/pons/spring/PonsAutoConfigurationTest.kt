package com.pons.spring

import com.pons.spring.testapp.TestApplication
import org.junit.jupiter.api.Assertions.assertNotNull
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.springframework.boot.autoconfigure.AutoConfigurations
import org.springframework.boot.test.context.runner.WebApplicationContextRunner
import java.nio.file.Files

class PonsAutoConfigurationTest {
    private val runner = WebApplicationContextRunner()
        .withUserConfiguration(TestApplication::class.java)
        .withConfiguration(AutoConfigurations.of(PonsSpringAutoConfiguration::class.java))

    @Test
    fun requestValidationIsOnByDefaultAndCanBeDisabled() {
        runner.run { context ->
            assertTrue(
                context.getBeanNamesForType(PonsRequestValidationFilter::class.java).isEmpty(),
                "the filter is registered through a FilterRegistrationBean",
            )
            assertTrue(
                context.containsBean("ponsRequestValidationFilter"),
                "a servlet application validates its declared requests by default",
            )
        }
        runner.withPropertyValues("pons.validation.enabled=false").run { context ->
            assertTrue(!context.containsBean("ponsRequestValidationFilter"))
        }
    }

    @Test
    fun exportAndEndpointAreOffByDefault() {
        runner.run { context ->
            assertTrue(context.getBeanNamesForType(PonsExportRunner::class.java).isEmpty())
            assertTrue(context.getBeanNamesForType(PonsIrEndpoint::class.java).isEmpty())
        }
    }

    @Test
    fun configuredExportWritesTheDocument() {
        val path = Files.createTempFile("pons-spring-auto", ".json")
        runner.withPropertyValues(
            "pons.export.path=$path",
            "pons.info.title=Auto Configured",
        ).run { context ->
            val export = context.getBean(PonsExportRunner::class.java)
            export.export()
            assertTrue(Files.readString(path).contains("\"Auto Configured\""))
            assertNull(context.getBeanProvider(PonsIrEndpoint::class.java).getIfAvailable())
        }
    }

    @Test
    fun configuredEndpointServesTheDocument() {
        runner.withPropertyValues("pons.endpoint.enabled=true").run { context ->
            val endpoint = context.getBean(PonsIrEndpoint::class.java)
            assertNotNull(endpoint.document())
            assertTrue(endpoint.document().contains("\"ir_version\""))
        }
    }
}
