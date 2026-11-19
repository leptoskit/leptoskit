
# Kept while [native.android] push is on. The notifications plugin compiles
# without Firebase, so it reaches these Firebase and Google Play services
# names by reflection, and R8 renames or removes names nothing refers to in
# code.
-keep class com.google.firebase.messaging.FirebaseMessaging {
    public static *** getInstance();
    public *** getToken();
}
-keep class com.google.android.gms.tasks.Task {
    public *** addOnSuccessListener(com.google.android.gms.tasks.OnSuccessListener);
    public *** addOnFailureListener(com.google.android.gms.tasks.OnFailureListener);
}
-keep interface com.google.android.gms.tasks.OnSuccessListener { *; }
-keep interface com.google.android.gms.tasks.OnFailureListener { *; }
