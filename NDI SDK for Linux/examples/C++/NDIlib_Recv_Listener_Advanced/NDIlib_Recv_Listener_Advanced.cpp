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

static std::mutex  g_print_mtx;
static std::mutex  g_receiver_uuid_mtx;
static std::string g_receiver_uuid;
static std::string g_receiver_name;

static void print(const char* fmt, ...)
{
	std::lock_guard<std::mutex> print_lock(g_print_mtx);
	va_list args;
	va_start(args, fmt);
	vprintf(fmt, args);
	va_end(args);
}

// NOTE: If it seems that you are not receiving any monitoring events from the receiver, then double check
// that the provided vendor ID is correct.
static void recv_listener_monitoring(std::atomic<bool>& running, NDIlib_recv_listener_instance_t pNDI_recv_listener)
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
		if (const NDIlib_recv_listener_event* p_events = NDIlib_recv_listener_get_events(pNDI_recv_listener, &num_events, timeout_in_ms)) {
			for (uint32_t i = 0; i != num_events; i++) {
				print("event[%s]: %s=%s\n", p_events[i].p_uuid, p_events[i].p_name, p_events[i].p_value);
			}

			// Be sure to free the events.
			NDIlib_recv_listener_free_events(pNDI_recv_listener, p_events);
		}
	}
}

// NOTE: If it seems that you are not successfully triggering any commands to the receiver, then double check
// that the provided vendor ID is correct.
static void recv_listener_commanding(std::atomic<bool>& running, NDIlib_recv_listener_instance_t pNDI_recv_listener)
{
	std::random_device random_device;
	std::default_random_engine random_engine(random_device());
	std::chrono::steady_clock::time_point last_command_time = std::chrono::steady_clock::now();
	std::string receiver_name, receiver_uuid;

	NDIlib_find_instance_t pNDI_find = NDIlib_find_create_v2();
	if (!pNDI_find) {
		print("The finder failed to create.\n");
		return;
	}

	while (running) {
		// Wait for a small period of time.
		std::this_thread::sleep_for(std::chrono::milliseconds(500));

		const std::chrono::steady_clock::time_point curr_time = std::chrono::steady_clock::now();

		// Only send a command every five seconds.
		if (curr_time - last_command_time >= std::chrono::seconds(5)) {
			last_command_time = curr_time;

			if (receiver_uuid.empty()) {
				// Copy the string containing the receiver UUID, which comes from a different thread. The
				// receiver list cannot be reliably shared across threads.
				std::unique_lock<std::mutex> receiver_uuid_lock(g_receiver_uuid_mtx);
				receiver_name = g_receiver_name;
				receiver_uuid = g_receiver_uuid;
				receiver_uuid_lock.unlock();

				if (!receiver_uuid.empty()) {
					// Get the updated list of sources.
					uint32_t num_sources = 0;
					const NDIlib_source_t* p_sources = NDIlib_find_get_current_sources(pNDI_find, &num_sources);

					if (p_sources) {
						std::uniform_int_distribution<uint32_t> source_dist(0, num_sources - 1);

						// Choose a random source to connect to.
						const NDIlib_source_t* p_source = p_sources + source_dist(random_engine);
						if (p_source->p_ndi_name) {
							// Issue a connect command to the chosen receiver.
							NDIlib_recv_listener_send_connect(pNDI_recv_listener, receiver_uuid.c_str(), p_source->p_ndi_name);
							print("Sent connect command to %s: %s.\n", receiver_name.c_str(), p_source->p_ndi_name);
						}
					}
				}
			} else {
				// Issue a disconnect command to the receiver UUID that we previously sent a connect command to.
				NDIlib_recv_listener_send_connect(pNDI_recv_listener, receiver_uuid.c_str(), nullptr);
				print("Sent disconnect command to %s.\n", receiver_name.c_str());
				receiver_uuid = "";
			}
		}
	}

	// Destroy the NDI finder
	NDIlib_find_destroy(pNDI_find);
}

int main(int argc, char* argv[])
{
	// Not required, but "correct" (see the SDK documentation).
	if (!NDIlib_initialize()) {
		return 0;
	}

	// Create an instance of the receiver listener.
	NDIlib_recv_listener_instance_t pNDI_recv_listener = NDIlib_recv_listener_create(nullptr);
	if (!pNDI_recv_listener) {
		print("The receiver listener failed to create. Please configure the connection to the NDI discovery server.\n");
		NDIlib_destroy();
		return 0;
	}

	// To remember what our last connected state in order to know when the connection state changes.
	bool last_connected = false;

	// Launch the thread that monitors events from subscribed receivers.
	std::atomic<bool> running{true};
	std::thread monitoring_thread(&recv_listener_monitoring, std::ref(running), pNDI_recv_listener);
	std::thread commanding_thread(&recv_listener_commanding, std::ref(running), pNDI_recv_listener);

	// Run for five minutes.
	using namespace std::chrono;
	for (const auto start = high_resolution_clock::now(); high_resolution_clock::now() - start < minutes(5);) {
		// Check to see if the listener is currently connected.
		bool curr_connected = NDIlib_recv_listener_is_connected(pNDI_recv_listener);

		// Has the connection state changed?
		if (last_connected != curr_connected) {
			print("The listener is now %s.\n", curr_connected ? "connected" : "disconnected");
			last_connected = curr_connected;
		}

		if (!NDIlib_recv_listener_wait_for_receivers(pNDI_recv_listener, 1000)) {
			print("No change to the receivers found.\n");
			continue;
		}

		// Get the updated list of receivers.
		uint32_t num_receivers = 0;
		const NDIlib_receiver_t* p_receivers = NDIlib_recv_listener_get_receivers(pNDI_recv_listener, &num_receivers);

		// Display all of the found receivers.
		print("Network receivers (%u found).\n", num_receivers);
		for (uint32_t i = 0; i != num_receivers; i++) {
			print("%u. %s\n", i + 1, p_receivers[i].p_name);

			// If this is a receiver that we are not currently subscribed to, then subscribe to it.
			if (!p_receivers[i].events_subscribed) {
				NDIlib_recv_listener_subscribe_events(pNDI_recv_listener, p_receivers[i].p_uuid);
			}
		}

		std::unique_lock<std::mutex> receiver_uuid_lock(g_receiver_uuid_mtx);
		if (p_receivers && num_receivers != 0) {
			// Copy the first receiver's name and UUID.
			g_receiver_name = p_receivers[0].p_name ? p_receivers[0].p_name : "";
			g_receiver_uuid = p_receivers[0].p_uuid ? p_receivers[0].p_uuid : "";
		} else {
			// There are no known receivers, ensure the commanding thread isn't issuing commands to a receiver.
			g_receiver_name = "";
			g_receiver_uuid = "";
		}
		receiver_uuid_lock.unlock();
	}

	// Stop the monitoring thread.
	running = false;

	// Wait for the threads to finish.
	commanding_thread.join();
	monitoring_thread.join();

	// Destroy the receiver advertiser.
	NDIlib_recv_listener_destroy(pNDI_recv_listener);

	// Clean up the initialization.
	NDIlib_destroy();
	return 0;
}
