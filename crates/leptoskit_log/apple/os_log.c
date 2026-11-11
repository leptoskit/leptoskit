// The unified log on iOS and macOS. Every message is marked public: the
// runtime writes its own diagnostics, never user data, and a private message
// reads as `<private>` on a device that is not attached to a debugger.
#include <os/log.h>
#include <stdint.h>

void *leptoskit_os_log_create(const char *subsystem, const char *category) {
	return (void *)os_log_create(subsystem, category);
}

void leptoskit_os_log_write(void *log, uint8_t type, const char *message) {
	os_log_with_type((os_log_t)log, (os_log_type_t)type, "%{public}s", message);
}
