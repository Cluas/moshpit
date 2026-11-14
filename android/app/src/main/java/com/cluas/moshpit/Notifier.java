package com.cluas.moshpit;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.os.Build;

/**
 * The app's notifications (`docs/nsh/DESIGN.md` §3.3, 推送到点开 pane): two channels — {@code agents} (an agent on
 * a host needs the person, high importance) and {@code sessions} (a session's state, low) — and posting one whose tap
 * opens the app at {@code moshpit://connection/<id>?pane=<pane>}.
 * <p>
 * Remote push comes later: on iOS the relay forwards sealed envelopes to APNs; on Android it will forward them to FCM,
 * and Firebase's messaging service will open them ({@code moshpit_seal::receive}) and post them here. {@link PushHook}
 * is where that service plugs in; this build has none (no Google Play services linked). What linking it takes — the
 * Firebase config, the token in the pairing, the envelope's path, the GPL §7 permission for LICENSE — is
 * android/PUSH.md.
 */
final class Notifier {
    static final String AGENTS = "agents";
    static final String SESSIONS = "sessions";

    /** Where a push service plugs in: its token goes to the relay's pairing, its messages to {@link #post}. */
    interface PushHook {
        /** The device's push token, once the service has one (null: none, no push on this build). */
        String token();
    }

    /** This build's: no push service. */
    static final PushHook NO_PUSH = () -> null;

    private Notifier() {}

    static void createChannels(Context context) {
        if (Build.VERSION.SDK_INT < 26) return;
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (manager == null) return;
        NotificationChannel agents = new NotificationChannel(AGENTS,
                context.getString(R.string.channel_agents), NotificationManager.IMPORTANCE_HIGH);
        agents.setDescription(context.getString(R.string.channel_agents_description));
        NotificationChannel sessions = new NotificationChannel(SESSIONS,
                context.getString(R.string.channel_sessions), NotificationManager.IMPORTANCE_LOW);
        sessions.setDescription(context.getString(R.string.channel_sessions_description));
        manager.createNotificationChannel(agents);
        manager.createNotificationChannel(sessions);
    }

    static void post(Context context, String channel, String tag, String title, String body, String link) {
        NotificationManager manager = (NotificationManager) context.getSystemService(Context.NOTIFICATION_SERVICE);
        if (manager == null) return;
        Intent open = new Intent(Intent.ACTION_VIEW, Uri.parse(link), context, MainActivity.class);
        open.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        PendingIntent tap = PendingIntent.getActivity(context, tag.hashCode(), open,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        Notification.Builder builder = Build.VERSION.SDK_INT >= 26
                ? new Notification.Builder(context, channel)
                : new Notification.Builder(context);
        Notification notification = builder
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(title)
                .setContentText(body)
                .setStyle(new Notification.BigTextStyle().bigText(body))
                .setContentIntent(tap)
                .setAutoCancel(true)
                .build();
        manager.notify(tag, 1, notification);
    }
}
