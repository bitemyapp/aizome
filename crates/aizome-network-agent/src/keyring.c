/* A small libsecret bridge. NetworkManager's established attributes let
 * Aizome read credentials saved by other desktops without migrating them.
 * This runs in a disposable worker process; the agent can cancel even an
 * outstanding keyring unlock prompt by terminating that worker. */
#include <libsecret/secret.h>

static const SecretSchema schema = {
    .name = "org.freedesktop.NetworkManager.Connection",
    .flags = SECRET_SCHEMA_DONT_MATCH_NAME,
    .attributes = {
        { "connection-uuid", SECRET_SCHEMA_ATTRIBUTE_STRING },
        { "setting-name", SECRET_SCHEMA_ATTRIBUTE_STRING },
        { "setting-key", SECRET_SCHEMA_ATTRIBUTE_STRING },
        { NULL, 0 }
    }
};

typedef void (*Emit)(void *, const char *, const char *);

int aizome_keyring(const char *action, const char *uuid, const char *setting,
                   const char *key, const char *value, const char *label,
                   int unlock, Emit emit, void *context) {
    GError *error = NULL;
    GHashTable *attrs = secret_attributes_build(&schema, "connection-uuid", uuid, NULL);
    if (*setting)
        g_hash_table_insert(attrs, g_strdup("setting-name"), g_strdup(setting));
    if (*key)
        g_hash_table_insert(attrs, g_strdup("setting-key"), g_strdup(key));
    if (g_str_equal(action, "lookup")) {
        SecretSearchFlags flags = SECRET_SEARCH_ALL | SECRET_SEARCH_LOAD_SECRETS;
        if (unlock)
            flags |= SECRET_SEARCH_UNLOCK;
        GList *items = secret_service_search_sync(NULL, &schema, attrs, flags, NULL, &error);
        for (GList *it = items; it; it = it->next) {
            SecretItem *item = it->data;
            GHashTable *attributes = secret_item_get_attributes(item);
            const char *name = g_hash_table_lookup(attributes, "setting-key");
            SecretValue *secret = secret_item_get_secret(item);
            const char *text = secret ? secret_value_get_text(secret) : NULL;
            if (name && text)
                emit(context, name, text);
            if (secret)
                secret_value_unref(secret);
            g_hash_table_unref(attributes);
        }
        g_list_free_full(items, g_object_unref);
    } else if (g_str_equal(action, "store")) {
        secret_password_storev_sync(&schema, attrs, SECRET_COLLECTION_DEFAULT,
                                    label, value, NULL, &error);
    } else if (g_str_equal(action, "delete")) {
        secret_password_clearv_sync(&schema, attrs, NULL, &error);
    }
    g_hash_table_unref(attrs);
    if (error) {
        /* Do not log credentials or connection data. */
        g_error_free(error);
        return 1;
    }
    return 0;
}
