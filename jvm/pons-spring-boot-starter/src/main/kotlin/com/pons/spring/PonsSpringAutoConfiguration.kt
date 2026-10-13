package com.pons.spring

import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper
import com.pons.ir.Info
import org.springframework.beans.factory.ObjectProvider
import org.springframework.boot.autoconfigure.AutoConfiguration
import org.springframework.boot.autoconfigure.condition.ConditionalOnMissingBean
import org.springframework.boot.autoconfigure.condition.ConditionalOnProperty
import org.springframework.boot.autoconfigure.condition.ConditionalOnWebApplication
import org.springframework.boot.context.properties.EnableConfigurationProperties
import org.springframework.context.ApplicationContext
import org.springframework.context.annotation.Bean

@AutoConfiguration
@ConditionalOnWebApplication(type = ConditionalOnWebApplication.Type.SERVLET)
@EnableConfigurationProperties(PonsProperties::class)
class PonsSpringAutoConfiguration {
    @Bean
    @ConditionalOnMissingBean
    fun ponsExporter(
        context: ApplicationContext,
        mapper: ObjectProvider<ObjectMapper>,
        properties: PonsProperties,
        lowerers: ObjectProvider<PonsDocumentLowerer>,
    ): PonsExporter {
        val title = properties.info.title
            ?.takeIf { it.isNotBlank() }
            ?: context.environment.getProperty("spring.application.name")
            ?: "Application"
        return PonsExporter(
            context = context,
            mapper = mapper.getIfAvailable { jacksonObjectMapper() },
            info = Info(
                title = title,
                version = properties.info.version,
                description = properties.info.description?.takeIf { it.isNotBlank() },
                servers = properties.info.servers,
            ),
            defaultAuthScheme = properties.auth.defaultScheme?.takeIf { it.isNotBlank() },
            lowerers = lowerers.orderedStream().toList(),
        )
    }

    @Bean
    @ConditionalOnMissingBean
    @ConditionalOnProperty(prefix = "pons.export", name = ["path"])
    fun ponsExportRunner(
        exporter: PonsExporter,
        properties: PonsProperties,
    ): PonsExportRunner = PonsExportRunner(
        exporter = exporter,
        path = checkNotNull(properties.export.path),
        exit = properties.export.exit,
    )

    @Bean
    @ConditionalOnMissingBean
    @ConditionalOnProperty(prefix = "pons.endpoint", name = ["enabled"], havingValue = "true")
    fun ponsIrEndpoint(exporter: PonsExporter): PonsIrEndpoint = PonsIrEndpoint(exporter)
}
