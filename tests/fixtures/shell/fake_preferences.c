// Qt uses the preferences daemon, which has its own HOME. For the macOS
// profile test only, supply the fake HOME as the preferences container.
// This keeps Qt's native settings backend and its organization/domain logic.
#include <CoreFoundation/CoreFoundation.h>
#include <stdlib.h>

extern CFPropertyListRef _CFPreferencesCopyValueWithContainer(CFStringRef, CFStringRef, CFStringRef, CFStringRef, CFStringRef);
extern CFArrayRef _CFPreferencesCopyKeyListWithContainer(CFStringRef, CFStringRef, CFStringRef, CFStringRef);
extern void _CFPreferencesSetValueWithContainer(CFStringRef, CFPropertyListRef, CFStringRef, CFStringRef, CFStringRef, CFStringRef);
extern Boolean _CFPreferencesSynchronizeWithContainer(CFStringRef, CFStringRef, CFStringRef, CFStringRef);
static CFStringRef container(void) {
    return CFAutorelease(CFStringCreateWithCString(NULL, getenv("HOME"), kCFStringEncodingUTF8));
}

static CFPropertyListRef read_value(CFStringRef key, CFStringRef app, CFStringRef user, CFStringRef host) {
    return _CFPreferencesCopyValueWithContainer(key, app, user, host, container());
}
static CFArrayRef read_keys(CFStringRef app, CFStringRef user, CFStringRef host) {
    return _CFPreferencesCopyKeyListWithContainer(app, user, host, container());
}
static void write_value(CFStringRef key, CFPropertyListRef value, CFStringRef app, CFStringRef user, CFStringRef host) {
    _CFPreferencesSetValueWithContainer(key, value, app, user, host, container());
}
struct domain { CFStringRef app, user, host; };
static void put(const void *key, const void *value, void *context) {
    struct domain *domain = context;
    write_value(key, value, domain->app, domain->user, domain->host);
}
static void write_values(CFDictionaryRef values, CFArrayRef removed, CFStringRef app, CFStringRef user, CFStringRef host) {
    struct domain domain = {app, user, host};
    if (values) CFDictionaryApplyFunction(values, put, &domain);
    if (removed) for (CFIndex i = 0; i < CFArrayGetCount(removed); ++i)
        write_value(CFArrayGetValueAtIndex(removed, i), NULL, app, user, host);
}
static Boolean sync_domain(CFStringRef app, CFStringRef user, CFStringRef host) {
    return _CFPreferencesSynchronizeWithContainer(app, user, host, container());
}

__attribute__((used, section("__DATA,__interpose")))
static const struct { const void *replacement, *original; } redirects[] = {
    {read_value, CFPreferencesCopyValue}, {read_keys, CFPreferencesCopyKeyList},
    {write_value, CFPreferencesSetValue}, {write_values, CFPreferencesSetMultiple},
    {sync_domain, CFPreferencesSynchronize}
};
