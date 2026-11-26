# kotlinx.serialization: keep generated serializers for the API models.
-keepclassmembers @kotlinx.serialization.Serializable class com.zalom3ui.app.data.** {
    *** Companion;
    kotlinx.serialization.KSerializer serializer(...);
}
-keep,includedescriptorclasses class com.zalom3ui.app.data.**$$serializer { *; }

# JNI native entry points: keep class names and native method signatures for libzalocore.so and libzaloopus.so.
# ZaloCore's natives are looked up by name (Java_com_zalom3ui_app_server_ZaloCore_*), so keep them unrenamed.
-keep,includedescriptorclasses class com.zalom3ui.app.server.ZaloCore {
    native <methods>;
}
-keep,includedescriptorclasses class com.zalom3ui.app.call.OpusEncoder {
    native <methods>;
}
-keep,includedescriptorclasses class com.zalom3ui.app.call.OpusDecoder {
    native <methods>;
}
