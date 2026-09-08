-keep class com.example.irohapp.IrohBridge { *; }

-keep interface com.example.irohapp.IrohTransferListener { *; }

-keepclassmembers class * implements com.example.irohapp.IrohTransferListener {
    void onTransferProgress(int, int, java.lang.String);
    void onModelMetadata(java.lang.String, java.lang.String);
    void onFetchComplete(java.lang.String);
}
