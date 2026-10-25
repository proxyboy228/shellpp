package dev.shellpp.android

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/** The app's logic around the core's answers (M1): no device, no core. */
class CoreLogicTest {
    @Test
    fun anOkAnswerGivesItsValueAndAnErrorThrowsItsCode() {
        val ok = Core.answer("""{"ok":{"state":"locked","remember_device_available":true,"remember_device_enabled":false,"remember_device_lost":false}}""")
        val status = LockStatus.from(ok as JSONObject)
        assertEquals("locked", status.state)
        assertTrue(status.rememberAvailable)
        assertFalse(status.unlocked)
        assertNull(Core.answer("""{"ok":null}"""))
        assertEquals(true, Core.answer("""{"ok":true}"""))
        try {
            Core.answer("""{"error":{"code":"too_many_attempts","message":"too many attempts","details":{"retry_after_secs":5}}}""")
            fail("an error answer throws")
        } catch (error: CoreException) {
            assertEquals("too_many_attempts", error.code)
            assertEquals(5, (error.details as JSONObject).getInt("retry_after_secs"))
        }
    }

    @Test
    fun userAtHostWithAnOptionalPortIsAQuickConnect() {
        assertEquals(QuickTarget("root", "203.0.113.5", null), quickTarget("root@203.0.113.5"))
        assertEquals(QuickTarget("deploy", "web.example.org", 2222), quickTarget(" deploy@web.example.org:2222 "))
        assertNull(quickTarget("web.example.org"))
        assertNull(quickTarget("a@b@c"))
        assertNull(quickTarget("user@host:99999"))
        assertNull(quickTarget("user name@host"))
    }

    @Test
    fun theStrengthIsAdviceFromLengthAndVariety() {
        assertEquals(0, passwordStrength(""))
        assertEquals(1, passwordStrength("abcdefg"))
        assertTrue(passwordStrength("aaaaaaaa") in 2..3)
        assertEquals(4, passwordStrength("Correct-Horse-Battery-9"))
    }

    @Test
    fun aRecordReadsItsTitleAndTheGroupsUsername() {
        val host = Record(
            JSONObject(
                """{"type":"host","vault_id":"v","record_id":"h","data":{"address":"web.example.org","label":null,"ssh":{}},
                   "inherited":{"username":{"value":"deploy","from_group_id":"g"}},"secrets_set":[]}""",
            ),
        )
        assertFalse(host.isGroup)
        assertEquals("web.example.org", host.title)
        assertEquals("deploy", host.username)
        val group = Record(JSONObject("""{"type":"group","vault_id":"v","record_id":"g","data":{"name":"Servers","color":"ocean"},"host_count":3}"""))
        assertTrue(group.isGroup)
        assertEquals("Servers", group.title)
        assertEquals(3, group.hostCount)
        assertEquals("ocean", group.color)
    }

    @Test
    fun theWeeklyPasswordIsDueAfterAWeekOrWhenTheClockWentBack() {
        val day = 24L * 60 * 60 * 1000
        val now = 1_800_000_000_000L
        assertFalse(Fingerprint.passwordDue(now - day, now))
        assertTrue(Fingerprint.passwordDue(now - 8 * day, now))
        assertTrue("a clock set back", Fingerprint.passwordDue(now + 30 * day, now))
    }
}
