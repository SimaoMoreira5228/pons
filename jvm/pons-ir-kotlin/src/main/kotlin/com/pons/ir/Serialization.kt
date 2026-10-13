package com.pons.ir

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.json.Json

object PonsIrJson {
    @OptIn(ExperimentalSerializationApi::class)
    val instance: Json = Json {
        explicitNulls = false
        encodeDefaults = false
        ignoreUnknownKeys = false
    }
}
