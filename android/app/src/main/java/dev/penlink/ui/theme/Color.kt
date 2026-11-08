package dev.penlink.ui.theme

import androidx.compose.material3.ColorScheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Immutable
import androidx.compose.ui.graphics.Color

/*
 * Fallback palette, used when Material You colours are off or unavailable. The accent is an "ink" violet (the pen)
 * with a teal highlight (the link); both pass WCAG AA against their "on" colours.
 */

internal val LightScheme: ColorScheme = lightColorScheme(
    primary = Color(0xFF5143D1),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFFE4DFFF),
    onPrimaryContainer = Color(0xFF150062),
    inversePrimary = Color(0xFFC7BFFF),
    secondary = Color(0xFF5E5B7A),
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFE4E0F9),
    onSecondaryContainer = Color(0xFF1B1930),
    tertiary = Color(0xFF006A66),
    onTertiary = Color(0xFFFFFFFF),
    tertiaryContainer = Color(0xFF9DF2E8),
    onTertiaryContainer = Color(0xFF00201E),
    background = Color(0xFFFBF8FF),
    onBackground = Color(0xFF1B1B21),
    surface = Color(0xFFFBF8FF),
    onSurface = Color(0xFF1B1B21),
    surfaceVariant = Color(0xFFE5E1EC),
    onSurfaceVariant = Color(0xFF47464F),
    surfaceTint = Color(0xFF5143D1),
    inverseSurface = Color(0xFF303036),
    inverseOnSurface = Color(0xFFF2EFF7),
    error = Color(0xFFBA1A1A),
    onError = Color(0xFFFFFFFF),
    errorContainer = Color(0xFFFFDAD6),
    onErrorContainer = Color(0xFF410002),
    outline = Color(0xFF78767F),
    outlineVariant = Color(0xFFC8C5D0),
    scrim = Color(0xFF000000),
    surfaceBright = Color(0xFFFBF8FF),
    surfaceDim = Color(0xFFDBD9E0),
    surfaceContainerLowest = Color(0xFFFFFFFF),
    surfaceContainerLow = Color(0xFFF5F2FA),
    surfaceContainer = Color(0xFFEFECF4),
    surfaceContainerHigh = Color(0xFFE9E6EE),
    surfaceContainerHighest = Color(0xFFE4E1E9),
)

internal val DarkScheme: ColorScheme = darkColorScheme(
    primary = Color(0xFFC7BFFF),
    onPrimary = Color(0xFF2A1799),
    primaryContainer = Color(0xFF3E30B5),
    onPrimaryContainer = Color(0xFFE4DFFF),
    inversePrimary = Color(0xFF5143D1),
    secondary = Color(0xFFC8C3E3),
    onSecondary = Color(0xFF302E49),
    secondaryContainer = Color(0xFF464461),
    onSecondaryContainer = Color(0xFFE4E0F9),
    tertiary = Color(0xFF55DBD0),
    onTertiary = Color(0xFF003733),
    tertiaryContainer = Color(0xFF00504C),
    onTertiaryContainer = Color(0xFF9DF2E8),
    background = Color(0xFF121218),
    onBackground = Color(0xFFE4E1E9),
    surface = Color(0xFF121218),
    onSurface = Color(0xFFE4E1E9),
    surfaceVariant = Color(0xFF47464F),
    onSurfaceVariant = Color(0xFFC8C5D0),
    surfaceTint = Color(0xFFC7BFFF),
    inverseSurface = Color(0xFFE4E1E9),
    inverseOnSurface = Color(0xFF303036),
    error = Color(0xFFFFB4AB),
    onError = Color(0xFF690005),
    errorContainer = Color(0xFF93000A),
    onErrorContainer = Color(0xFFFFDAD6),
    outline = Color(0xFF928F99),
    outlineVariant = Color(0xFF47464F),
    scrim = Color(0xFF000000),
    surfaceBright = Color(0xFF38383F),
    surfaceDim = Color(0xFF121218),
    surfaceContainerLowest = Color(0xFF0D0D13),
    surfaceContainerLow = Color(0xFF1B1B21),
    surfaceContainer = Color(0xFF1F1F25),
    surfaceContainerHigh = Color(0xFF292930),
    surfaceContainerHighest = Color(0xFF34343B),
)

/** Colours Material has no slot for: online/success and warning states. Fixed hues so state reads the same with Material You. */
@Immutable
data class ExtraColors(
    val success: Color,
    val onSuccess: Color,
    val successContainer: Color,
    val onSuccessContainer: Color,
    val warning: Color,
    val onWarningContainer: Color,
    val warningContainer: Color,
)

internal val LightExtraColors = ExtraColors(
    success = Color(0xFF1B7F3B),
    onSuccess = Color(0xFFFFFFFF),
    successContainer = Color(0xFFC5F0CF),
    onSuccessContainer = Color(0xFF00210B),
    warning = Color(0xFF8A5100),
    onWarningContainer = Color(0xFF2D1600),
    warningContainer = Color(0xFFFFDCBE),
)

internal val DarkExtraColors = ExtraColors(
    success = Color(0xFF7BD98F),
    onSuccess = Color(0xFF00391A),
    successContainer = Color(0xFF0F5226),
    onSuccessContainer = Color(0xFFC5F0CF),
    warning = Color(0xFFFFB870),
    onWarningContainer = Color(0xFFFFDCBE),
    warningContainer = Color(0xFF693C00),
)
