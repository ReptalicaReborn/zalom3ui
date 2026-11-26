package com.zalom3ui.app.ui

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialExpressiveTheme
import androidx.compose.material3.MotionScheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.zalom3ui.app.data.Appearance
import com.zalom3ui.app.data.ChatAccent
import com.zalom3ui.app.data.ThemeMode

private val BaseTypography = Typography()

val ExpressiveTypography = Typography(
    displayLarge = BaseTypography.displayLarge,
    displayMedium = BaseTypography.displayMedium,
    displaySmall = BaseTypography.displaySmall,
    headlineLarge = BaseTypography.headlineLarge.copy(
        fontWeight = FontWeight.Bold,
        letterSpacing = (-0.5).sp,
    ),
    headlineMedium = BaseTypography.headlineMedium.copy(
        fontWeight = FontWeight.Bold,
        letterSpacing = (-0.4).sp,
    ),
    headlineSmall = BaseTypography.headlineSmall.copy(
        fontWeight = FontWeight.SemiBold,
        letterSpacing = (-0.2).sp,
    ),
    titleLarge = BaseTypography.titleLarge.copy(
        fontWeight = FontWeight.SemiBold,
        letterSpacing = (-0.2).sp,
    ),
    titleMedium = BaseTypography.titleMedium.copy(
        fontWeight = FontWeight.SemiBold,
    ),
    titleSmall = BaseTypography.titleSmall,
    bodyLarge = BaseTypography.bodyLarge.copy(
        fontSize = 15.5.sp,
        lineHeight = 21.sp,
    ),
    bodyMedium = BaseTypography.bodyMedium,
    bodySmall = BaseTypography.bodySmall,
    labelLarge = BaseTypography.labelLarge,
    labelMedium = BaseTypography.labelMedium.copy(
        fontFeatureSettings = "tnum",
    ),
    labelSmall = BaseTypography.labelSmall.copy(
        fontFeatureSettings = "tnum",
    ),
)

val ExpressiveShapes = Shapes(
    extraSmall = RoundedCornerShape(6.dp),
    small = RoundedCornerShape(12.dp),
    medium = RoundedCornerShape(18.dp),
    large = RoundedCornerShape(24.dp),
    extraLarge = RoundedCornerShape(32.dp),
)

@Immutable
private data class AccentPalette(
    val swatch: Color,
    val lightPrimary: Color,
    val lightOnPrimary: Color,
    val lightPrimaryContainer: Color,
    val lightOnPrimaryContainer: Color,
    val lightSecondary: Color,
    val lightOnSecondary: Color,
    val lightSecondaryContainer: Color,
    val lightOnSecondaryContainer: Color,
    val lightOutgoing: Color,
    val lightOnOutgoing: Color,
    val lightTintedBg: Color,
    val darkPrimary: Color,
    val darkOnPrimary: Color,
    val darkPrimaryContainer: Color,
    val darkOnPrimaryContainer: Color,
    val darkSecondary: Color,
    val darkOnSecondary: Color,
    val darkSecondaryContainer: Color,
    val darkOnSecondaryContainer: Color,
    val darkOutgoing: Color,
    val darkOnOutgoing: Color,
    val darkTintedBg: Color,
)

private val AccentPalettes: Map<ChatAccent, AccentPalette> = mapOf(
    ChatAccent.ULTRAMARINE to AccentPalette(
        swatch = Color(0xFF2C6BED),
        lightPrimary = Color(0xFF2C6BED),
        lightOnPrimary = Color(0xFFFFFFFF),
        lightPrimaryContainer = Color(0xFFDCE5FF),
        lightOnPrimaryContainer = Color(0xFF00194A),
        lightSecondary = Color(0xFF535E78),
        lightOnSecondary = Color(0xFFFFFFFF),
        lightSecondaryContainer = Color(0xFFDCE3FA),
        lightOnSecondaryContainer = Color(0xFF101B32),
        lightOutgoing = Color(0xFF2C6BED),
        lightOnOutgoing = Color(0xFFFFFFFF),
        lightTintedBg = Color(0xFFF3F6FD),
        darkPrimary = Color(0xFFB3C7FF),
        darkOnPrimary = Color(0xFF002B75),
        darkPrimaryContainer = Color(0xFF1B44A8),
        darkOnPrimaryContainer = Color(0xFFDCE5FF),
        darkSecondary = Color(0xFFBCC6E0),
        darkOnSecondary = Color(0xFF263047),
        darkSecondaryContainer = Color(0xFF3C4760),
        darkOnSecondaryContainer = Color(0xFFDCE3FA),
        darkOutgoing = Color(0xFF2C6BED),
        darkOnOutgoing = Color(0xFFFFFFFF),
        darkTintedBg = Color(0xFF11151E),
    ),
    ChatAccent.CRIMSON to AccentPalette(
        swatch = Color(0xFFC93B4B),
        lightPrimary = Color(0xFFC93B4B),
        lightOnPrimary = Color(0xFFFFFFFF),
        lightPrimaryContainer = Color(0xFFFFDADE),
        lightOnPrimaryContainer = Color(0xFF40000D),
        lightSecondary = Color(0xFF765659),
        lightOnSecondary = Color(0xFFFFFFFF),
        lightSecondaryContainer = Color(0xFFFFDADE),
        lightOnSecondaryContainer = Color(0xFF2C1518),
        lightOutgoing = Color(0xFFC93B4B),
        lightOnOutgoing = Color(0xFFFFFFFF),
        lightTintedBg = Color(0xFFFDF4F5),
        darkPrimary = Color(0xFFFFB2B9),
        darkOnPrimary = Color(0xFF670019),
        darkPrimaryContainer = Color(0xFF911B2D),
        darkOnPrimaryContainer = Color(0xFFFFDADE),
        darkSecondary = Color(0xFFE5BDC0),
        darkOnSecondary = Color(0xFF44292C),
        darkSecondaryContainer = Color(0xFF5C3F42),
        darkOnSecondaryContainer = Color(0xFFFFDADE),
        darkOutgoing = Color(0xFFB53242),
        darkOnOutgoing = Color(0xFFFFFFFF),
        darkTintedBg = Color(0xFF1C1214),
    ),
    ChatAccent.FOREST to AccentPalette(
        swatch = Color(0xFF1E7D54),
        lightPrimary = Color(0xFF1E7D54),
        lightOnPrimary = Color(0xFFFFFFFF),
        lightPrimaryContainer = Color(0xFFC4F1D7),
        lightOnPrimaryContainer = Color(0xFF002113),
        lightSecondary = Color(0xFF4E6356),
        lightOnSecondary = Color(0xFFFFFFFF),
        lightSecondaryContainer = Color(0xFFD0E8D8),
        lightOnSecondaryContainer = Color(0xFF0B1F15),
        lightOutgoing = Color(0xFF1E7D54),
        lightOnOutgoing = Color(0xFFFFFFFF),
        lightTintedBg = Color(0xFFF3F9F5),
        darkPrimary = Color(0xFF8CD9B0),
        darkOnPrimary = Color(0xFF003822),
        darkPrimaryContainer = Color(0xFF0E5E3D),
        darkOnPrimaryContainer = Color(0xFFC4F1D7),
        darkSecondary = Color(0xFFB4CCBC),
        darkOnSecondary = Color(0xFF203529),
        darkSecondaryContainer = Color(0xFF364B3F),
        darkOnSecondaryContainer = Color(0xFFD0E8D8),
        darkOutgoing = Color(0xFF1B704B),
        darkOnOutgoing = Color(0xFFFFFFFF),
        darkTintedBg = Color(0xFF101914),
    ),
    ChatAccent.PLUM to AccentPalette(
        swatch = Color(0xFF7B46B8),
        lightPrimary = Color(0xFF7B46B8),
        lightOnPrimary = Color(0xFFFFFFFF),
        lightPrimaryContainer = Color(0xFFEEDCFF),
        lightOnPrimaryContainer = Color(0xFF2B0053),
        lightSecondary = Color(0xFF645A70),
        lightOnSecondary = Color(0xFFFFFFFF),
        lightSecondaryContainer = Color(0xFFEBDEF7),
        lightOnSecondaryContainer = Color(0xFF20182A),
        lightOutgoing = Color(0xFF7B46B8),
        lightOnOutgoing = Color(0xFFFFFFFF),
        lightTintedBg = Color(0xFFF8F4FC),
        darkPrimary = Color(0xFFD9B9FF),
        darkOnPrimary = Color(0xFF471082),
        darkPrimaryContainer = Color(0xFF612C9D),
        darkOnPrimaryContainer = Color(0xFFEEDCFF),
        darkSecondary = Color(0xFFCFC1DB),
        darkOnSecondary = Color(0xFF352D40),
        darkSecondaryContainer = Color(0xFF4C4357),
        darkOnSecondaryContainer = Color(0xFFEBDEF7),
        darkOutgoing = Color(0xFF7340AE),
        darkOnOutgoing = Color(0xFFFFFFFF),
        darkTintedBg = Color(0xFF18121E),
    ),
    ChatAccent.TEAL to AccentPalette(
        swatch = Color(0xFF0E7C86),
        lightPrimary = Color(0xFF0E7C86),
        lightOnPrimary = Color(0xFFFFFFFF),
        lightPrimaryContainer = Color(0xFFBDF0F5),
        lightOnPrimaryContainer = Color(0xFF002023),
        lightSecondary = Color(0xFF4A6366),
        lightOnSecondary = Color(0xFFFFFFFF),
        lightSecondaryContainer = Color(0xFFCDE7EA),
        lightOnSecondaryContainer = Color(0xFF051F22),
        lightOutgoing = Color(0xFF0E7C86),
        lightOnOutgoing = Color(0xFFFFFFFF),
        lightTintedBg = Color(0xFFF2F9FA),
        darkPrimary = Color(0xFF83D4DD),
        darkOnPrimary = Color(0xFF00363C),
        darkPrimaryContainer = Color(0xFF005D66),
        darkOnPrimaryContainer = Color(0xFFBDF0F5),
        darkSecondary = Color(0xFFB1CBCE),
        darkOnSecondary = Color(0xFF1C3437),
        darkSecondaryContainer = Color(0xFF334B4E),
        darkOnSecondaryContainer = Color(0xFFCDE7EA),
        darkOutgoing = Color(0xFF0E707A),
        darkOnOutgoing = Color(0xFFFFFFFF),
        darkTintedBg = Color(0xFF10191A),
    ),
    ChatAccent.SLATE to AccentPalette(
        swatch = Color(0xFF4B5A6E),
        lightPrimary = Color(0xFF4B5A6E),
        lightOnPrimary = Color(0xFFFFFFFF),
        lightPrimaryContainer = Color(0xFFD6E3F5),
        lightOnPrimaryContainer = Color(0xFF0F1D2E),
        lightSecondary = Color(0xFF565F6D),
        lightOnSecondary = Color(0xFFFFFFFF),
        lightSecondaryContainer = Color(0xFFDAE3F0),
        lightOnSecondaryContainer = Color(0xFF131C28),
        lightOutgoing = Color(0xFF4B5A6E),
        lightOnOutgoing = Color(0xFFFFFFFF),
        lightTintedBg = Color(0xFFF4F6F9),
        darkPrimary = Color(0xFFB7C7DC),
        darkOnPrimary = Color(0xFF213042),
        darkPrimaryContainer = Color(0xFF39485C),
        darkOnPrimaryContainer = Color(0xFFD6E3F5),
        darkSecondary = Color(0xFFBEC7D5),
        darkOnSecondary = Color(0xFF28313E),
        darkSecondaryContainer = Color(0xFF3F4755),
        darkOnSecondaryContainer = Color(0xFFDAE3F0),
        darkOutgoing = Color(0xFF445367),
        darkOnOutgoing = Color(0xFFFFFFFF),
        darkTintedBg = Color(0xFF13161B),
    ),
)

private fun accentPalette(accent: ChatAccent): AccentPalette =
    AccentPalettes[accent] ?: AccentPalettes.getValue(ChatAccent.ULTRAMARINE)

/** Signature swatch color shown for a [ChatAccent] option in Settings. */
fun accentSwatchColor(accent: ChatAccent): Color = accentPalette(accent).swatch

// A calm messenger look: plain white (or near-black) surfaces, one strong accent for actions and your own bubbles.
private val LightColors = lightColorScheme(
    primary = Color(0xFF2C6BED),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFFDCE5FF),
    onPrimaryContainer = Color(0xFF00194A),
    secondary = Color(0xFF535E78),
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFDCE3FA),
    onSecondaryContainer = Color(0xFF101B32),
    tertiary = Color(0xFF735471),
    onTertiary = Color(0xFFFFFFFF),
    tertiaryContainer = Color(0xFFFED7F9),
    onTertiaryContainer = Color(0xFF2A122B),
    error = Color(0xFFBA1A1A),
    onError = Color(0xFFFFFFFF),
    errorContainer = Color(0xFFFFDAD6),
    onErrorContainer = Color(0xFF410002),
    background = Color(0xFFFFFFFF),
    onBackground = Color(0xFF1B1B1F),
    surface = Color(0xFFFFFFFF),
    onSurface = Color(0xFF1B1B1F),
    surfaceVariant = Color(0xFFE2E2EC),
    onSurfaceVariant = Color(0xFF5E5F67),
    outline = Color(0xFF767680),
    outlineVariant = Color(0xFFC6C6D0),
    surfaceContainerLowest = Color(0xFFFFFFFF),
    surfaceContainerLow = Color(0xFFF6F6FA),
    surfaceContainer = Color(0xFFF0F0F5),
    surfaceContainerHigh = Color(0xFFEAEAEF),
    surfaceContainerHighest = Color(0xFFE4E4E9),
)

private val DarkColors = darkColorScheme(
    primary = Color(0xFFB3C7FF),
    onPrimary = Color(0xFF002B75),
    primaryContainer = Color(0xFF1B44A8),
    onPrimaryContainer = Color(0xFFDCE5FF),
    secondary = Color(0xFFBCC6E0),
    onSecondary = Color(0xFF263047),
    secondaryContainer = Color(0xFF3C4760),
    onSecondaryContainer = Color(0xFFDCE3FA),
    tertiary = Color(0xFFE1BBDC),
    onTertiary = Color(0xFF412741),
    tertiaryContainer = Color(0xFF5A3D58),
    onTertiaryContainer = Color(0xFFFED7F9),
    error = Color(0xFFFFB4AB),
    onError = Color(0xFF690005),
    errorContainer = Color(0xFF93000A),
    onErrorContainer = Color(0xFFFFDAD6),
    background = Color(0xFF121214),
    onBackground = Color(0xFFE4E2E6),
    surface = Color(0xFF121214),
    onSurface = Color(0xFFE4E2E6),
    surfaceVariant = Color(0xFF45464F),
    onSurfaceVariant = Color(0xFFA9AAB4),
    outline = Color(0xFF90909A),
    outlineVariant = Color(0xFF45464F),
    surfaceContainerLowest = Color(0xFF0D0D0F),
    surfaceContainerLow = Color(0xFF1A1A1D),
    surfaceContainer = Color(0xFF1E1E21),
    surfaceContainerHigh = Color(0xFF28282B),
    surfaceContainerHighest = Color(0xFF333336),
)

// "ReptDoc gold": the dark warm palette of the diyhrtvn site. A dark-only theme.
private val GoldBg = Color(0xFF14120F)
private val GoldSurface = Color(0xFF1A1714)
private val GoldSurface2 = Color(0xFF221E1A)
private val GoldLine = Color(0xFF2E2923)
private val GoldLine2 = Color(0xFF3D362E)
private val GoldInk = Color(0xFFEAE1D9)
private val GoldInk2 = Color(0xFFC9C0B5)
private val GoldMuted = Color(0xFF8C8378)
private val Gold = Color(0xFFE6C275)
private val GoldOn = Color(0xFF1A1206)
private val GoldSoft = Color(0xFF2A2419) // rgba(230,194,117,0.1) over the base, flattened
private val GoldWarn = Color(0xFFE07A5F)

private val GoldColors = darkColorScheme(
    primary = Gold,
    onPrimary = GoldOn,
    primaryContainer = GoldSoft,
    onPrimaryContainer = Gold,
    secondary = GoldInk2,
    onSecondary = GoldOn,
    secondaryContainer = GoldSurface2,
    onSecondaryContainer = GoldInk,
    tertiary = Color(0xFFD9A98B),
    onTertiary = GoldOn,
    tertiaryContainer = GoldSurface2,
    onTertiaryContainer = GoldInk,
    error = GoldWarn,
    onError = Color(0xFF2B0E06),
    errorContainer = Color(0xFF4A2218),
    onErrorContainer = Color(0xFFFFDAD0),
    background = GoldBg,
    onBackground = GoldInk,
    surface = GoldBg,
    onSurface = GoldInk,
    surfaceVariant = GoldSurface2,
    onSurfaceVariant = GoldMuted,
    outline = GoldLine2,
    outlineVariant = GoldLine,
    surfaceContainerLowest = Color(0xFF0F0D0B),
    surfaceContainerLow = GoldSurface,
    surfaceContainer = GoldSurface,
    surfaceContainerHigh = GoldSurface2,
    surfaceContainerHighest = Color(0xFF2A2520),
)

// 14dp corners like the site; buttons stay pills.
private val GoldShapes = Shapes(
    extraSmall = RoundedCornerShape(8.dp),
    small = RoundedCornerShape(10.dp),
    medium = RoundedCornerShape(14.dp),
    large = RoundedCornerShape(18.dp),
    extraLarge = RoundedCornerShape(28.dp),
)

private val OutgoingBlue = Color(0xFF2C6BED)

/** Colors of the conversation screen that Material's color scheme has no slot for. */
@Immutable
data class ChatColors(
    val background: Color,
    val outgoing: Color,
    val onOutgoing: Color,
    val incoming: Color,
    val onIncoming: Color,
    val dark: Boolean,
)

val LocalChatColors = staticCompositionLocalOf {
    ChatColors(
        background = Color.White,
        outgoing = OutgoingBlue,
        onOutgoing = Color.White,
        incoming = Color(0xFFEBECEF),
        onIncoming = Color(0xFF1B1B1F),
        dark = false,
    )
}

/** Whether the app is drawn dark, from the theme setting or the system. */
@Composable
fun isAppInDarkTheme(appearance: Appearance): Boolean = when (appearance.themeMode) {
    ThemeMode.SYSTEM -> isSystemInDarkTheme()
    ThemeMode.LIGHT -> false
    ThemeMode.DARK, ThemeMode.REPTDOC_GOLD -> true
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun AppTheme(appearance: Appearance = Appearance(), content: @Composable () -> Unit) {
    val dark = isAppInDarkTheme(appearance)
    val gold = appearance.themeMode == ThemeMode.REPTDOC_GOLD
    val dynamic = !gold && appearance.dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
    val context = LocalContext.current
    val palette = accentPalette(appearance.accent)
    val colors: ColorScheme = when {
        gold -> GoldColors
        dynamic && dark -> dynamicDarkColorScheme(context)
        dynamic -> dynamicLightColorScheme(context)
        dark -> DarkColors.copy(
            primary = palette.darkPrimary,
            onPrimary = palette.darkOnPrimary,
            primaryContainer = palette.darkPrimaryContainer,
            onPrimaryContainer = palette.darkOnPrimaryContainer,
            secondary = palette.darkSecondary,
            onSecondary = palette.darkOnSecondary,
            secondaryContainer = palette.darkSecondaryContainer,
            onSecondaryContainer = palette.darkOnSecondaryContainer,
        )
        else -> LightColors.copy(
            primary = palette.lightPrimary,
            onPrimary = palette.lightOnPrimary,
            primaryContainer = palette.lightPrimaryContainer,
            onPrimaryContainer = palette.lightOnPrimaryContainer,
            secondary = palette.lightSecondary,
            onSecondary = palette.lightOnSecondary,
            secondaryContainer = palette.lightSecondaryContainer,
            onSecondaryContainer = palette.lightOnSecondaryContainer,
        )
    }
    val chat = when {
        gold -> ChatColors(
            background = GoldBg,
            outgoing = Gold,
            onOutgoing = GoldOn,
            incoming = GoldSurface2,
            onIncoming = GoldInk,
            dark = true,
        )
        dynamic -> ChatColors(
            background = if (appearance.tintedBackground) {
                lerp(colors.background, colors.primary, if (dark) 0.07f else 0.045f)
            } else {
                colors.background
            },
            outgoing = colors.primary,
            onOutgoing = colors.onPrimary,
            incoming = colors.surfaceContainerHigh,
            onIncoming = colors.onSurface,
            dark = dark,
        )
        dark -> ChatColors(
            background = if (appearance.tintedBackground) palette.darkTintedBg else DarkColors.background,
            outgoing = palette.darkOutgoing,
            onOutgoing = palette.darkOnOutgoing,
            incoming = Color(0xFF2B2C30),
            onIncoming = Color(0xFFE4E4E7),
            dark = true,
        )
        else -> ChatColors(
            background = if (appearance.tintedBackground) palette.lightTintedBg else LightColors.background,
            outgoing = palette.lightOutgoing,
            onOutgoing = palette.lightOnOutgoing,
            incoming = Color(0xFFEBECEF),
            onIncoming = Color(0xFF1B1B1F),
            dark = false,
        )
    }
    CompositionLocalProvider(
        LocalChatColors provides chat,
        LocalUltraExpressive provides appearance.ultraExpressive,
    ) {
        MaterialExpressiveTheme(
            colorScheme = colors,
            motionScheme = MotionScheme.expressive(),
            shapes = if (gold) GoldShapes else ExpressiveShapes,
            typography = ExpressiveTypography,
            content = content,
        )
    }
}

/** Easter egg: true when "4x faster mode" (hilariously oversized M3 Expressive buttons) is enabled. */
val LocalUltraExpressive = staticCompositionLocalOf { false }

/** A letter avatar's colors: a soft tint behind a strong letter of the same hue. */
@Immutable
data class AvatarColors(val container: Color, val content: Color)

private val AvatarPalette = listOf(
    AvatarColors(Color(0xFFDCE5FD), Color(0xFF1F4FD1)), // blue
    AvatarColors(Color(0xFFD6ECF6), Color(0xFF0B6A96)), // sky
    AvatarColors(Color(0xFFCFEDE6), Color(0xFF0B6F5C)), // teal
    AvatarColors(Color(0xFFD9EDD3), Color(0xFF2E7A1E)), // green
    AvatarColors(Color(0xFFECEBCB), Color(0xFF6B6512)), // olive
    AvatarColors(Color(0xFFFCEBC8), Color(0xFF8A5A00)), // amber
    AvatarColors(Color(0xFFFDDFD0), Color(0xFFB24A12)), // orange
    AvatarColors(Color(0xFFFBD9DA), Color(0xFFB3261E)), // red
    AvatarColors(Color(0xFFF9D9EA), Color(0xFFA8216E)), // pink
    AvatarColors(Color(0xFFEBDDFB), Color(0xFF6D3CC4)), // purple
    AvatarColors(Color(0xFFE0E0F8), Color(0xFF4A4AB8)), // indigo
    AvatarColors(Color(0xFFDEE1E6), Color(0xFF4B5563)), // slate
)

/** The same person or group always gets the same colors. */
fun avatarColors(key: String): AvatarColors = AvatarPalette[key.hashCode().mod(AvatarPalette.size)]

/** A group member's name color on an incoming bubble, readable in both themes. */
@Composable
fun senderNameColor(key: String): Color {
    val colors = avatarColors(key)
    return if (LocalChatColors.current.dark) colors.container else colors.content
}
