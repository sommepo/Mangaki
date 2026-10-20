# R8 rules for the Mangaki release build.
#
# Every class in the app package is reached by name from outside the Java code, so R8 must not
# rename, merge or remove any of it:
#  - Rust looks classes up with ClassLoader.loadClass() and calls static methods by name/signature
#    over JNI (DocumentScanner, SecureStore, OnDeviceOcr; see fork_api.rs and the OCR crate) and
#    starts WebviewActivity by its class name (lib.rs).
#  - WebView calls the @JavascriptInterface methods of the bridge object by name.
#  - Activities, the service and the provider are referenced from the manifest.
# The app's own code is tiny; shrinking and optimising the libraries (ML Kit, Play services) is
# where the size/startup win comes from.
-keep class dev.local.mangareader.** { *; }

# WebView only exposes bridge methods that still carry the runtime-visible annotation.
-keepattributes RuntimeVisibleAnnotations,RuntimeVisibleParameterAnnotations,AnnotationDefault
-keepattributes Signature,InnerClasses,EnclosingMethod,Exceptions
-keepclassmembers class * {
    @android.webkit.JavascriptInterface <methods>;
}

# Readable native crash/ANR stack traces for the app's own frames.
-keepattributes SourceFile,LineNumberTable

# Compile-time-only annotation packages referenced by Google libraries.
-dontwarn com.google.errorprone.annotations.**
-dontwarn javax.annotation.**
-dontwarn org.checkerframework.**
-dontwarn com.google.j2objc.annotations.**

# The bundled recognizer's native pipeline calls back into ML Kit classes over JNI. Keep ML Kit
# itself intact (its own consumer rules should suffice, but OCR is a core feature and cannot be
# exercised in CI); R8 still shrinks the remaining Play-services/Firebase-components dex.
-keep class com.google.mlkit.** { *; }
-keep class com.google.android.gms.internal.mlkit_** { *; }
