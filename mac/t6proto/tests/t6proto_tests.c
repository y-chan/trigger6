#include "t6proto.h"

#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static uint32_t read_le32(const uint8_t *in)
{
	return (uint32_t)in[0] | ((uint32_t)in[1] << 8) |
	       ((uint32_t)in[2] << 16) | ((uint32_t)in[3] << 24);
}

static void test_bulk_header_matches_capture_shape(void)
{
	uint8_t header[T6_BULK_DMA_HEADER_SIZE];

	t6_build_display_bulk_header(header, 0x87e0, 0x03000000, 0x87e0, 0,
				     false);

	assert(read_le32(header + 0) == 0);
	assert(read_le32(header + 4) == 0x87e0);
	assert(read_le32(header + 8) == 0x03000000);
	assert(read_le32(header + 12) == 0x87e0);
	assert(read_le32(header + 16) == 0);
	assert(header[20] == 0);
}

static void test_fragmentation_sets_continue_flag(void)
{
	uint8_t header[T6_BULK_DMA_HEADER_SIZE];
	uint32_t total = 0x4f7e0;
	uint32_t max_packet = 0x19000;

	assert(t6_fragment_count(total, max_packet) == 4);
	assert(t6_fragment_size(total, max_packet, 0) == 0x19000);
	assert(t6_fragment_size(total, max_packet, 1) == 0x19000);
	assert(t6_fragment_size(total, max_packet, 2) == 0x19000);
	assert(t6_fragment_size(total, max_packet, 3) == 0x47e0);
	assert(t6_fragment_has_more(total, max_packet, 0));
	assert(t6_fragment_has_more(total, max_packet, 1));
	assert(t6_fragment_has_more(total, max_packet, 2));
	assert(!t6_fragment_has_more(total, max_packet, 3));

	t6_build_display_bulk_header(
		header, total, 0x030b7dc0, t6_fragment_size(total, max_packet, 1),
		t6_fragment_offset(max_packet, 1),
		t6_fragment_has_more(total, max_packet, 1));

	assert(read_le32(header + 12) == 0x19000);
	assert(read_le32(header + 16) == 0x19000);
	assert(header[20] == T6_PACKET_FLAG_CONTINUE_TO_SEND);
}

static void test_jpeg_flip_header(void)
{
	struct t6_video_flip_header source = t6_make_jpeg_flip_header(
		0, 0x87e0 - T6_VIDEO_FLIP_HEADER_SIZE, 1360, 768, 0x03000000,
		0x50055005, 0);
	uint8_t header[T6_VIDEO_FLIP_HEADER_SIZE];

	t6_write_video_flip_header(header, &source);

	assert(read_le32(header + 0) == T6_VIDEO_CMD_FLIP_PRIMARY);
	assert(read_le32(header + 12) == T6_VIDEO_COLOR_NV12);
	assert(read_le32(header + 20) == 0x50055005);
	assert(read_le32(header + 32) == T6_VIDEO_COLOR_JPEG);
	assert(header[47] == 0);
}

static void test_interrupt_parsing(void)
{
	uint8_t packet[T6_INTERRUPT_PACKET_SIZE];
	struct t6_display_interrupt interrupt;

	memset(packet, 0, sizeof(packet));
	packet[0] = T6_INT_FUNC_DISPLAY;
	packet[12] = 0x3b;
	packet[19] = T6_INT_DISPLAY_EVENT_FENCE_ID;

	interrupt = t6_parse_display_interrupt(packet);

	assert(interrupt.is_display);
	assert(interrupt.display_data == 0x3b);
	assert(interrupt.display_event == T6_INT_DISPLAY_EVENT_FENCE_ID);
	assert(interrupt.has_fence_id);
	assert(!interrupt.has_jpeg_error);
}

int main(void)
{
	test_bulk_header_matches_capture_shape();
	test_fragmentation_sets_continue_flag();
	test_jpeg_flip_header();
	test_interrupt_parsing();

	puts("t6proto tests passed");
	return 0;
}

