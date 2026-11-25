# kotlinx.serialization: keep generated serializers for the API models.
-keepclassmembers @kotlinx.serialization.Serializable class com.zalom3ui.app.data.** {
    *** Companion;
    kotlinx.serialization.KSerializer serializer(...);
}
-keep,includedescriptorclasses class com.zalom3ui.app.data.**$$serializer { *; }

# JNI native entry points: keep class names and native method signatures for libnodebridge.so and libzaloopus.so.
-keep,includedescriptorclasses class com.zalom3ui.app.server.NodeRuntime {
    native <methods>;
}
-keep,includedescriptorclasses class com.zalom3ui.app.call.OpusEncoder {
    native <methods>;
}
-keep,includedescriptorclasses class com.zalom3ui.app.call.OpusDecoder {
    native <methods>;
}
