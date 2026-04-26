#include <csignal>
#include <cstddef>
#include <cstdlib>
#include <cstdio>
#include <atomic>

#ifdef _WIN32
#include <windows.h>

#ifdef _WIN64
#pragma comment(lib, "Processing.NDI.Lib.x64.lib")
#else // _WIN64
#pragma comment(lib, "Processing.NDI.Lib.x86.lib")
#endif // _WIN64

#endif

#include <Processing.NDI.Lib.h>

#define MINIAUDIO_IMPLEMENTATION
#ifdef __APPLE__
#define MA_NO_RUNTIME_LINKING
#endif
#include "../ThirdParty/miniaudio.h"

static std::atomic<bool> exit_loop(false);
static void sigint_handler(int)
{
	exit_loop = true;
}

int main(int argc, char* argv[])
{
	if (argc < 2) {
		printf("Audio file not specified\n");
		return 0;
	}

	// Setup the miniaudio decoder configuration. We want 32-bit floating point audio, however we'll use the
	// sample rate and number of channels as specified by the audio file.
	const ma_decoder_config audio_decoder_config = ma_decoder_config_init(ma_format_f32, 0, 0);

	// Use miniaudio to read the specified audio file.
	ma_decoder audio_decoder = {};
	if (ma_decoder_init_file(argv[1], &audio_decoder_config, &audio_decoder) == MA_SUCCESS) {
		ma_format audio_fmt;
		ma_uint32 num_channels, sample_rate;
		ma_decoder_get_data_format(&audio_decoder, &audio_fmt, &num_channels, &sample_rate, nullptr, 0);

		// Not required, but "correct" (see the SDK documentation).
		if (NDIlib_initialize()) {
			// Catch interrupt so that we can shut down gracefully
			signal(SIGINT, sigint_handler);

			// Create an NDI source that is called "My Audio" and is clocked to the audio.
			NDIlib_send_create_t NDI_send_create_desc;
			NDI_send_create_desc.p_ndi_name = "My Audio";
			NDI_send_create_desc.clock_audio = true;

			// We create the NDI sender.
			NDIlib_send_instance_t pNDI_send = NDIlib_send_create(&NDI_send_create_desc);
			if (pNDI_send) {
				const int max_samples_per_frame = 1920;

				// Setup the audio frame for 32-bit floating-point with the channels interleaved.
				NDIlib_audio_frame_interleaved_32f_t NDI_audio_frame;
				NDI_audio_frame.sample_rate = sample_rate;
				NDI_audio_frame.no_channels = num_channels;
				NDI_audio_frame.p_data = (float*)malloc(max_samples_per_frame * NDI_audio_frame.no_channels * sizeof(float));

				while (!exit_loop) {
					// Read the next audio frame from the file.
					ma_uint64 num_samples;
					ma_result ret = ma_decoder_read_pcm_frames(&audio_decoder, NDI_audio_frame.p_data, max_samples_per_frame, &num_samples);
					if (ret != MA_SUCCESS) {
						// Have we reached the end of the file? If so, loop back to the beginning.
						if (ret == MA_AT_END && ma_decoder_seek_to_pcm_frame(&audio_decoder, 0) == MA_SUCCESS)
							continue;

						break;
					}

					// Set the number of samples to be what was given back from the audio decoder.
					NDI_audio_frame.no_samples = (int)num_samples;

					// We now submit the frame. Note that this call will be clocked so that we end up
					// submitting at exactly at the sample rate.
					NDIlib_util_send_send_audio_interleaved_32f(pNDI_send, &NDI_audio_frame);
				}

				// Release the audio data.
				free(NDI_audio_frame.p_data);

				// Destroy the NDI sender.
				NDIlib_send_destroy(pNDI_send);
			}

			// Clean up the NDI library.
			NDIlib_destroy();
		} else {
			// Cannot run NDI. Most likely because the CPU is not sufficient (see SDK documentation). You can
			// check this directly with a call to NDIlib_is_supported_CPU().
			printf("Cannot run NDI.");
		}

		ma_decoder_uninit(&audio_decoder);
	} else {
		printf("Cannot initialize decoder for file: %s\n", argv[1]);
	}

	return 0;
}
