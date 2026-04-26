#include <cstdint>
#include <cstdio>
#include <chrono>
#include <Processing.NDI.Lib.h>

#ifdef _WIN32
#ifdef _WIN64
#pragma comment(lib, "Processing.NDI.Lib.x64.lib")
#else // _WIN64
#pragma comment(lib, "Processing.NDI.Lib.x86.lib")
#endif // _WIN64
#endif // _WIN32

int main(int argc, char* argv[])
{
	// Not required, but "correct" (see the SDK documentation).
	if (!NDIlib_initialize()) {
		return 0;
	}

	// Create an instance of the sender listener.
	NDIlib_send_listener_instance_t pNDI_send_listener = NDIlib_send_listener_create();
	if (!pNDI_send_listener) {
		printf("The sender listener failed to create. Please configure the connection to the NDI discovery server.\n");
		NDIlib_destroy();
		return 0;
	}

	// To remember what our last connected state in order to know when the connection state changes.
	bool last_connected = false;

	// Run for five minutes.
	using namespace std::chrono;
	for (const auto start = high_resolution_clock::now(); high_resolution_clock::now() - start < minutes(5);) {
		// Check to see if the listener is currently connected.
		bool curr_connected = NDIlib_send_listener_is_connected(pNDI_send_listener);

		// Has the connection state changed?
		if (last_connected != curr_connected) {
			printf("The listener is now %s.\n", curr_connected ? "connected" : "disconnected");
			last_connected = curr_connected;
		}

		if (!NDIlib_send_listener_wait_for_senders(pNDI_send_listener, 1000)) {
			printf("No change to the senders found.\n");
			continue;
		}

		// Get the updated list of senders.
		uint32_t num_senders = 0;
		const NDIlib_sender_t* p_senders = NDIlib_send_listener_get_senders(pNDI_send_listener, &num_senders);

		// Display all of the found senders.
		printf("Network senders (%u found).\n", num_senders);
		for (uint32_t i = 0; i != num_senders; i++) {
			printf("%u. %s\n", i + 1, p_senders[i].p_name);
		}
	}

	// Destroy the sender advertiser.
	NDIlib_send_listener_destroy(pNDI_send_listener);

	// Clean up the initialization.
	NDIlib_destroy();
	return 0;
}
