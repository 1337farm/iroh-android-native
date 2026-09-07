# JNI entry point is looked up by exact class/method name from native code.
-keep class com.example.irohapp.IrohBridge {
    *;
}

# Callbacks dispatched from native code (JNI GetMethodID) must survive R8.
-keep interface com.example.irohapp.IrohTransferListener {
    *;
}
-keepclassmembers class * implements com.example.irohapp.IrohTransferListener {
    public void onTransferProgress(int, int, java.lang.String);
    public void onModelMetadata(java.lang.String, java.lang.String);
    public void onFetchComplete(java.lang.String);
}
