#include <cstdarg>
#include <cstdint>
#include <cstdio>
#include <atomic>
#include <chrono>
#include <mutex>
#include <random>
#include <thread>
#include <Processing.NDI.Lib.h>

#ifdef _WIN32
#ifdef _WIN64
#pragma comment(lib, "Processing.NDI.Lib.x64.lib")
#else // _WIN64
#pragma comment(lib, "Processing.NDI.Lib.x86.lib")
#endif // _WIN64
#endif // _WIN32

static std::mutex g_print_mtx;

static void print(const char* fmt, ...)
{
	std::lock_guard<std::mutex> print_lock(g_print_mtx);
	va_list args;
	va_start(args, fmt);
	vprintf(fmt, args);
	va_end(args);
}

// NOTE: If it seems that you are not receiving any monitoring events from the sender, then double check
// that the provided vendor ID is correct.
static void send_listener_monitoring(std::atomic<bool>& running, NDIlib_send_listener_instance_t pNDI_send_listener)
{
	static const bool batch_process_events = true;
	static const uint32_t timeout_in_ms = batch_process_events ? 0 : 500;

	while (running) {
		if (batch_process_events) {
			// Slow ourselves down so that events can accumulate.
			std::this_thread::sleep_for(std::chrono::seconds(1));
		}

		// Retrieve any pending events.
		uint32_t num_events = 0;
		if (const NDIlib_send_listener_event* p_events = NDIlib_send_listener_get_events(pNDI_send_listener, &num_events, timeout_in_ms)) {
			for (uint32_t i = 0; i != num_events; i++) {
				print("event[%s]: %s=%s\n", p_events[i].p_uuid, p_events[i].p_name, p_events[i].p_value);
			}

			// Be sure to free the events.
			NDIlib_send_listener_free_events(pNDI_send_listener, p_events);
		}
	}
}

int main(int argc, char* argv[])
{
	// Not required, but "correct" (see the SDK documentation).
	if (!NDIlib_initialize()) {
		return 0;
	}

	// Create an instance of the sender listener.
	NDIlib_send_listener_instance_t pNDI_send_listener = NDIlib_send_listener_create(nullptr);
	if (!pNDI_send_listener) {
		print("The sender listener failed to create. Please configure the connection to the NDI discovery server.\n");
		NDIlib_destroy();
		return 0;
	}

	// To remember what our last connected state in order to know when the connection state changes.
	bool last_connected = false;

	// Launch the thread that monitors events from subscribed senders.
	std::atomic<bool> running{true};
	std::thread monitoring_thread(&send_listener_monitoring, std::ref(running), pNDI_send_listener);

	// Run for five minutes.
	using namespace std::chrono;
	for (const auto start = high_resolution_clock::now(); high_resolution_clock::now() - start < minutes(5);) {
		// Check to see if the listener is currently connected.
		bool curr_connected = NDIlib_send_listener_is_connected(pNDI_send_listener);

		// Has the connection state changed?
		if (last_connected != curr_connected) {
			print("The listener is now %s.\n", curr_connected ? "connected" : "disconnected");
			last_connected = curr_connected;
		}

		if (!NDIlib_send_listener_wait_for_senders(pNDI_send_listener, 1000)) {
			print("No change to the senders found.\n");
			continue;
		}

		// Get the updated list of senders.
		uint32_t num_senders = 0;
		const NDIlib_sender_t* p_senders = NDIlib_send_listener_get_senders(pNDI_send_listener, &num_senders);

		// Display all of the found senders.
		print("Network senders (%u found).\n", num_senders);
		for (uint32_t i = 0; i != num_senders; i++) {
			print("%u. %s\n", i + 1, p_senders[i].p_name);

			// If this is a sender that we are not currently subscribed to, then subscribe to it.
			if (!p_senders[i].events_subscribed) {
				NDIlib_send_listener_subscribe_events(pNDI_send_listener, p_senders[i].p_uuid);
			}
		}
	}

	// Stop the monitoring thread.
	running = false;

	// Wait for the threads to finish.
	monitoring_thread.join();

	// Destroy the sender advertiser.
	NDIlib_send_listener_destroy(pNDI_send_listener);

	// Clean up the initialization.
	NDIlib_destroy();
	return 0;
}
