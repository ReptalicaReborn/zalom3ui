package com.zalom3ui.app.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ApiClientTest {
    @Test
    fun normalizeServerUrl_addsSchemeAndTrimsSlash() {
        assertEquals("http://192.168.1.5:3000/", ApiClient.normalizeServerUrl(" 192.168.1.5:3000/ ").toString())
        assertEquals("https://zalo.example.com/", ApiClient.normalizeServerUrl("https://zalo.example.com").toString())
    }

    @Test
    fun normalizeServerUrl_rejectsBlankAndGarbage() {
        assertNull(ApiClient.normalizeServerUrl(""))
        assertNull(ApiClient.normalizeServerUrl("   "))
        assertNull(ApiClient.normalizeServerUrl("ftp://example.com"))
    }

    @Test
    fun serverSettings_onDeviceNeedsNoAddress() {
        assertTrue(ServerSettings(onDevice = true, serverUrl = "", token = "").isConfigured)
        assertFalse(ServerSettings(onDevice = false, serverUrl = "", token = "").isConfigured)
        assertTrue(ServerSettings(onDevice = false, serverUrl = "192.168.1.5:3000", token = "").isConfigured)
    }

    @Test
    fun eventsRequest_usesAuthorizationHeaderInsteadOfQueryParam() {
        val withToken = ApiClient("https://zalo.example.com", "secret-123").eventsRequest()
        assertEquals("Bearer secret-123", withToken.header("Authorization"))
        assertNull(withToken.url.queryParameter("token"))
        assertEquals("/ws", withToken.url.encodedPath)

        val withoutToken = ApiClient("http://127.0.0.1:45000", null).eventsRequest()
        assertNull(withoutToken.header("Authorization"))
        assertNull(withoutToken.url.queryParameter("token"))
    }

    @Test
    fun url_encodesEachSegmentWithoutSplittingOrDoubleEncoding() {
        val api = ApiClient("http://127.0.0.1:45000", "tok")
        val built = api.url("calls", "a/b c%20d", "answer")
        assertEquals("http://127.0.0.1:45000/api/calls/a%2Fb%20c%2520d/answer", built.toString())
    }
}
