package com.zalom3ui.app.ui

import com.zalom3ui.app.data.ApiClient
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.data.effectiveReducedMotion
import com.zalom3ui.app.data.isAnimatorScaleDisabled
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class ReducedMotionTest {
    @Test
    fun effectiveReducedMotion_isFalseOnlyWhenBothUserAndSystemAreOff() {
        assertFalse(effectiveReducedMotion(userPref = false, systemPref = false))
    }

    @Test
    fun effectiveReducedMotion_isTrueWhenUserToggleIsOn() {
        assertTrue(effectiveReducedMotion(userPref = true, systemPref = false))
    }

    @Test
    fun effectiveReducedMotion_isTrueWhenSystemSettingIsOn() {
        assertTrue(effectiveReducedMotion(userPref = false, systemPref = true))
    }

    @Test
    fun effectiveReducedMotion_isTrueWhenBothAreOn() {
        assertTrue(effectiveReducedMotion(userPref = true, systemPref = true))
    }

    @Test
    fun isAnimatorScaleDisabled_onlyTrueForZero() {
        assertTrue(isAnimatorScaleDisabled(0f))
        assertFalse(isAnimatorScaleDisabled(0.5f))
        assertFalse(isAnimatorScaleDisabled(1f))
        assertFalse(isAnimatorScaleDisabled(2f))
    }

    @Test
    fun settingsStoreKey_isDefined() {
        assertTrue(SettingsStore.KEY_REDUCED_MOTION.isNotBlank())
    }

    @Test
    fun apiClient_sharedHttpClientReusesSameInstanceAndPool() {
        val first = ApiClient.sharedHttpClient()
        val second = ApiClient.sharedHttpClient()
        assertSame(first, second)
        assertSame(first.connectionPool, second.connectionPool)
        assertSame(first.dispatcher, second.dispatcher)
    }
}
