#include <stdio.h>
#include <stdlib.h>
#include <string.h>

//#include "test_4kedid.c"
typedef unsigned int	UINT32;
typedef int				INT32;
typedef unsigned short 	UINT16, USHORT, *PUSHORT;
typedef unsigned char  	UINT8, UCHAR, *PUCHAR;
typedef unsigned long 	ULONG, *PULONG;

typedef enum{
	FALSE = 0,
	TRUE
}BOOLEAN;

#define DEBUG
#ifdef DEBUG
	#define DebugPrint(fmt, args...)   fprintf(stderr, fmt, ## args)
	#define MctPrint(fmt, args...)   fprintf(stderr, fmt, ## args)
#else
	#define DebugPrint(fmt, args...)
	#define MctPrint(fmt, args...)
#endif

#ifndef BIT_0
    #define BIT_0                               0x01
    #define BIT_1                               0x02
    #define BIT_2                               0x04
    #define BIT_3                               0x08
    #define BIT_4                               0x10
    #define BIT_5                               0x20
    #define BIT_6                               0x40
    #define BIT_7                               0x80
#endif

#define EDID_BLOCK_LENGTH                   128
#define EDID_EXTENSION_TAG                      0x02
#define EDID_EXTENSION_REVISION                 0x03
#define NUM_STANDARD_TIMINGS                    8
#define NUM_DETAILED_DESCRIPTORS                4
#define DETAILED_DESCRIPTOR_LENGTH              18
#define EDID_MONITOR_DESCRIPTOR_DATA_LENGTH     13

#define EDID_DATA_BLOCK_AUDIO                   0x01
#define EDID_DATA_BLOCK_VIDEO                   0x02
#define EDID_DATA_BLOCK_VENDOR_SPECIFIC         0x03
#define EDID_DATA_BLOCK_SPEAKER_ALLOCATION      0x04
#define EDID_DATA_BLOCK_VESA_DTC                0x05
#define EDID_DATA_BLOCK_USE_EXTENDED_TAG        0x07

// block 0
#define EDID_REG_TAG                            0x00
#define EDID_REG_ID_MANUFACTURER_NAME           0x08
#define EDID_REG_ID_PRODUCT_CODE                0x0A
#define EDID_REG_ID_SERIAL_NUMBER               0x0C
#define EDID_REG_MANUFACTURE_WEEK               0x10
#define EDID_REG_MANUFACTURE_YEAR               0x11
#define EDID_REG_VERSION_MAJOR                  0x12
#define EDID_REG_VERSION_MINOR                  0x13
#define EDID_REG_VIDEO_INPUT_DEFINITION         0x14
#define EDID_REG_FEATURE_SUPPORT                0x18
#define EDID_REG_ESTABLISHED_TIMING_1           0x23
#define EDID_REG_ESTABLISHED_TIMING_2           0x24
#define EDID_REG_ESTABLISHED_TIMING_3           0x25
#define EDID_REG_STANDARD_TIMING                0x26
#define EDID_REG_DETAILED_DESCRIPTOR            0x36
#define EDID_REG_NUM_EXTENSION_BLOCKS           0x7E
#define EDID_REG_CHECKSUM                       0x7F

// block 1
#define EDID_REG_EXTENSION_TAG                  0x00
#define EDID_REG_EXTENSION_REVISION             0x01
#define EDID_REG_LONG_DESCRIPTOR_OFFSET         0x02
#define EDID_REG_MISC_SUPPORT                   0x03
#define EDID_REG_DATA_START                     0x04

#define EDID_VIDEO_NONE                         0x00000000
#define EDID_VIDEO_INTERLACE                    0x00000001

typedef struct tEDIDVIDEODATAFORMAT
{
    UINT32      Flags;
    UINT16      Rate;
    UINT16      Reserved;
    UINT16      Horizontal;
    UINT16      Vertical;
    UINT16      Horizontal2;
    UINT16      Vertical2;
} EDIDVIDEODATAFORMAT, *PEDIDVIDEODATAFORMAT;


static EDIDVIDEODATAFORMAT g_VideoDataFormat[256] =
{
//                          Field         Hor-     Ver-                                                             Field       Picture     Pixel
//          Flags                 RSVD                                             VIC   Formats                                Aspect      Aspect
//                          Rate          izontal  tical                                                            Rate        Ratio       Ratio
//   ---------------------  ----- ----    ------- -------   ------- -------        ---  -----------------------  ------------- -------    -----------
    {EDID_VIDEO_INTERLACE,     0,    0,        0,      0,        0,      0},    //   0   No Video IdentificationCode Available
    {     EDID_VIDEO_NONE,    60,    0,      640,    480,        0,      0},    //   1  640x480p                59.94Hz/60Hz    4:3         1:1
    {     EDID_VIDEO_NONE,    60,    0,      720,    480,        0,      0},    //   2  720x480p                59.94Hz/60Hz    4:3         8:9
    {     EDID_VIDEO_NONE,    60,    0,      720,    480,        0,      0},    //   3  720x480p                59.94Hz/60Hz    16:9        32:27
    {     EDID_VIDEO_NONE,    60,    0,     1280,    720,        0,      0},    //   4  1280x720p               59.94Hz/60Hz    16:9        1:1
    {EDID_VIDEO_INTERLACE,    60,    0,     1920,   1080,        0,      0},    //   5  1920x1080i              59.94Hz/60Hz    16:9        1:1
    {EDID_VIDEO_INTERLACE,    60,    0,      720,    480,     1440,    480},    //   6  720(1440)x480i          59.94Hz/60Hz    4:3         8:9
    {EDID_VIDEO_INTERLACE,    60,    0,      720,    480,     1440,    480},    //   7  720(1440)x480i          59.94Hz/60Hz    16:9        32:27
    {     EDID_VIDEO_NONE,    60,    0,      720,    240,     1440,    240},    //   8  720(1440)x240p          59.94Hz/60Hz    4:3         4:9
    {     EDID_VIDEO_NONE,    60,    0,      720,    240,     1440,    240},    //   9  720(1440)x240p          59.94Hz/60Hz    16:9        16:27
    {EDID_VIDEO_INTERLACE,    60,    0,     2880,    480,        0,      0},    //  10  2880x480i               59.94Hz/60Hz    4:3         2:9 - 20:9
    {EDID_VIDEO_INTERLACE,    60,    0,     2880,    480,        0,      0},    //  11  2880x480i               59.94Hz/60Hz    16:9        8:27 -80:27
    {     EDID_VIDEO_NONE,    60,    0,     2880,    240,        0,      0},    //  12  2880x240p               59.94Hz/60Hz    4:3         1:9 -10:9
    {     EDID_VIDEO_NONE,    60,    0,     2880,    240,        0,      0},    //  13  2880x240p               59.94Hz/60Hz    16:9        4:27 - 40:27
    {     EDID_VIDEO_NONE,    60,    0,     1440,    480,        0,      0},    //  14  1440x480p               59.94Hz/60Hz    4:3         4:9 or 8:9
    {     EDID_VIDEO_NONE,    60,    0,     1440,    480,        0,      0},    //  15  1440x480p               59.94Hz/60Hz    16:9        16:27 or 32:27
    {     EDID_VIDEO_NONE,    60,    0,     1920,   1080,        0,      0},    //  16  1920x1080p              59.94Hz/60Hz    16:9        1:1
    {     EDID_VIDEO_NONE,    50,    0,      720,    576,        0,      0},    //  17  720x576p                50Hz             4:3        16:15
    {     EDID_VIDEO_NONE,    50,    0,      720,    576,        0,      0},    //  18  720x576p                50Hz            16:9        64:45
    {     EDID_VIDEO_NONE,    50,    0,     1280,    720,        0,      0},    //  19  1280x720p               50Hz            16:9        1:1
    {EDID_VIDEO_INTERLACE,    50,    0,     1920,   1080,        0,      0},    //  20  1920x1080i              50Hz            16:9        1:1
    {EDID_VIDEO_INTERLACE,    50,    0,      720,    576,     1440,    576},    //  21  720(1440)x576i          50Hz            4:3         16:15
    {EDID_VIDEO_INTERLACE,    50,    0,      720,    576,     1440,    576},    //  22  720(1440)x576i          50Hz            16:9        64:45
    {     EDID_VIDEO_NONE,    50,    0,      720,    288,     1440,    288},    //  23  720(1440)x288p          50Hz            4:3         8:15
    {     EDID_VIDEO_NONE,    50,    0,      720,    288,     1440,    288},    //  24  720(1440)x288p          50Hz            16:9        32:45
    {EDID_VIDEO_INTERLACE,    50,    0,     2880,    576,        0,      0},    //  25  2880x576i               50Hz            4:3         2:15 - 20:15
    {EDID_VIDEO_INTERLACE,    50,    0,     2880,    576,        0,      0},    //  26  2880x576i               50Hz            16:9        16:45-160:45
    {     EDID_VIDEO_NONE,    50,    0,     2880,    288,        0,      0},    //  27  2880x288p               50Hz            4:3         1:15-10:15
    {     EDID_VIDEO_NONE,    50,    0,     2880,    288,        0,      0},    //  28  2880x288p               50Hz            16:9        8:45 - 80:45
    {     EDID_VIDEO_NONE,    50,    0,     1440,    576,        0,      0},    //  29  1440x576p               50Hz            4:3         8:15 or 16:15
    {     EDID_VIDEO_NONE,    50,    0,     1440,    576,        0,      0},    //  30  1440x576p               50Hz            16:9        32:45 or 64:45
    {     EDID_VIDEO_NONE,    50,    0,     1920,   1080,        0,      0},    //  31  1920x1080p              50Hz            16:9        1:1
    {     EDID_VIDEO_NONE,    24,    0,     1920,   1080,        0,      0},    //  32  1920x1080p              23.97Hz/24Hz    16:9        1:1
    {     EDID_VIDEO_NONE,    25,    0,     1920,   1080,        0,      0},    //  33  1920x1080p              25Hz            16:9        1:1
    {     EDID_VIDEO_NONE,    30,    0,     1920,   1080,        0,      0},    //  34  1920x1080p              29.97Hz/30Hz    16:9        1:1
    {     EDID_VIDEO_NONE,    60,    0,     2880,    480,        0,      0},    //  35  2880x480p               59.94Hz/60Hz    4:3         2:9, 4:9, or 8:9
    {     EDID_VIDEO_NONE,    60,    0,     2880,    480,        0,      0},    //  36  2880x480p               59.94Hz/60Hz    16:9        8:27, 16:27, or 32:27
    {     EDID_VIDEO_NONE,    50,    0,     2880,    576,        0,      0},    //  37  2880x576p               50Hz            4:3         4:15, 8:15, or 16:15
    {     EDID_VIDEO_NONE,    50,    0,     2880,    576,        0,      0},    //  38  2880x576p               50Hz            16:9        16:45, 32:45, or 64:45
    {EDID_VIDEO_INTERLACE,    50,    0,     1920,   1080,        0,      0},    //  39  1920x1080i(1250 total)  50Hz            16:9        1:1
    {EDID_VIDEO_INTERLACE,   100,    0,     1920,   1080,        0,      0},    //  40  1920x1080i              100Hz           16:9        1:1
    {     EDID_VIDEO_NONE,   100,    0,     1280,    720,        0,      0},    //  41  1280x720p               100Hz           16:9        1:1
    {     EDID_VIDEO_NONE,   100,    0,      720,    576,        0,      0},    //  42  720x576p                100Hz           4:3         16:15
    {     EDID_VIDEO_NONE,   100,    0,      720,    576,        0,      0},    //  43  720x576p                100Hz           16:9        64:45
    {EDID_VIDEO_INTERLACE,   100,    0,      720,    576,     1440,    576},    //  44  720(1440)x576i          100Hz           4:3         16:15
    {EDID_VIDEO_INTERLACE,   100,    0,      720,    576,     1440,    576},    //  45  720(1440)x576i          100Hz           16:9        64:45
    {EDID_VIDEO_INTERLACE,   120,    0,     1920,   1080,        0,      0},    //  46  1920x1080i              119.88/120Hz    16:9        1:1
    {     EDID_VIDEO_NONE,   120,    0,     1280,    720,        0,      0},    //  47  1280x720p               119.88/120Hz    16:9        1:1
    {     EDID_VIDEO_NONE,   120,    0,      720,    480,        0,      0},    //  48  720x480p                119.88/120Hz    4:3         8:9
    {     EDID_VIDEO_NONE,   120,    0,      720,    480,        0,      0},    //  49  720x480p                119.88/120Hz    16:9        32:27
    {EDID_VIDEO_INTERLACE,   120,    0,      720,    480,     1440,    480},    //  50  720(1440)x480i          119.88/120Hz    4:3         8:9
    {EDID_VIDEO_INTERLACE,   120,    0,      720,    480,     1440,    480},    //  51  720(1440)x480i          119.88/120Hz    16:9        32:27
    {     EDID_VIDEO_NONE,   200,    0,      720,    576,        0,      0},    //  52  720x576p                200Hz           4:3         16:15
    {     EDID_VIDEO_NONE,   200,    0,      720,    576,        0,      0},    //  53  720x576p                200Hz           16:9        64:45
    {EDID_VIDEO_INTERLACE,   200,    0,      720,    576,     1440,    576},    //  54  720(1440)x576i          200Hz           4:3         16:15
    {EDID_VIDEO_INTERLACE,   200,    0,      720,    576,     1440,    576},    //  55  720(1440)x576i          200Hz           16:9        64:45
    {     EDID_VIDEO_NONE,   240,    0,      720,    480,        0,      0},    //  56  720x480p                239.76/240Hz    4:3         8:9
    {     EDID_VIDEO_NONE,   240,    0,      720,    480,        0,      0},    //  57  720x480p                239.76/240Hz    16:9        32:27
    {EDID_VIDEO_INTERLACE,   240,    0,      720,    480,     1440,    480},    //  58  720(1440)x480i          239.76/240Hz    4:3         8:9
    {EDID_VIDEO_INTERLACE,   240,    0,      720,    480,     1440,    480},    //  59  720(1440)x480i          239.76/240Hz    16:9        32:27
    {     EDID_VIDEO_NONE,    24,    0,     1280,    720,        0,      0},    //  60  1280x720p               23.97Hz/24Hz    16:9        1:1
    {     EDID_VIDEO_NONE,    25,    0,     1280,    720,        0,      0},    //  61  1280x720p               25Hz            16:9        1:1
    {     EDID_VIDEO_NONE,    30,    0,     1280,    720,        0,      0},    //  62  1280x720p               29.97Hz/30Hz    16:9        1:1
    {     EDID_VIDEO_NONE,   120,    0,     1920,   1080,        0,      0},    //  63  1920x1080p              119.88/120Hz    16:9        1:1
    {     EDID_VIDEO_NONE,   100,    0,     1920,   1080,        0,      0},    //  64  1920x1080p              100Hz           16:9        1:1
	{	  EDID_VIDEO_NONE,    24,    0,     1280,    720,        0,      0},    //  65  1280x720p               24Hz            64:27       4:3
	{     EDID_VIDEO_NONE,    25,    0,     1280,    720,        0,      0},    //  66  1280x720p               25Hz            64:27       4:3
	{     EDID_VIDEO_NONE,    30,    0,     1280,    720,        0,      0},    //  67  1280x720p               30Hz            64:27       4:3
	{     EDID_VIDEO_NONE,    50,    0,     1280,    720,        0,      0},    //  68  1280x720p               50Hz            64:27       4:3
	{     EDID_VIDEO_NONE,    60,    0,     1280,    720,        0,      0},    //  69  1280x720p               60Hz            64:27       4:3
	{     EDID_VIDEO_NONE,   100,    0,     1280,    720,        0,      0},    //  70  1280x720p              100Hz            64:27       4:3
	{     EDID_VIDEO_NONE,   120,    0,     1280,    720,        0,      0},    //  71  1280x720p              120Hz            64:27       4:3
	{     EDID_VIDEO_NONE,    24,    0,     1920,   1080,        0,      0},    //  72  1920x1080p              24Hz            64:27       4:3
	{     EDID_VIDEO_NONE,    25,    0,     1920,   1080,        0,      0},    //  73  1920x1080p              25Hz            64:27       4:3
	{     EDID_VIDEO_NONE,    30,    0,     1920,   1080,        0,      0},    //  74  1920x1080p              30Hz            64:27       4:3
	{     EDID_VIDEO_NONE,    50,    0,     1920,   1080,        0,      0},    //  75  1920x1080p              50Hz            64:27       4:3
	{     EDID_VIDEO_NONE,    60,    0,     1920,   1080,        0,      0},    //  76  1920x1080p              60Hz            64:27       4:3
	{     EDID_VIDEO_NONE,   100,    0,     1920,   1080,        0,      0},    //  77  1920x1080p             100Hz            64:27       4:3
	{     EDID_VIDEO_NONE,   120,    0,     1920,   1080,        0,      0},    //  78  1920x1080p             120Hz            64:27       4:3
	{     EDID_VIDEO_NONE,    24,    0,     1680,    720,        0,      0},    //  79  1680x720p               24Hz            64:27      64:63
	{     EDID_VIDEO_NONE,    25,    0,     1680,    720,        0,      0},    //  80  1680x720p               25Hz            64:27      64:63
	{     EDID_VIDEO_NONE,    30,    0,     1680,    720,        0,      0},    //  81  1680x720p               30Hz            64:27      64:63
	{     EDID_VIDEO_NONE,    50,    0,     1680,    720,        0,      0},    //  82  1680x720p               50Hz            64:27      64:63
	{     EDID_VIDEO_NONE,    60,    0,     1680,    720,        0,      0},    //  83  1680x720p               60Hz            64:27      64:63
	{     EDID_VIDEO_NONE,   100,    0,     1680,    720,        0,      0},    //  84  1680x720p              100Hz            64:27      64:63
	{     EDID_VIDEO_NONE,   120,    0,     1680,    720,        0,      0},    //  85  1680x720p              120Hz            64:27      64:63
	{     EDID_VIDEO_NONE,    24,    0,     2560,   1080,        0,      0},    //  86  2560x1080p              24Hz            64:27       1:1
	{     EDID_VIDEO_NONE,    25,    0,     2560,   1080,        0,      0},    //  87  2560x1080p              25Hz            64:27       1:1
	{     EDID_VIDEO_NONE,    30,    0,     2560,   1080,        0,      0},    //  88  2560x1080p              30Hz            64:27       1:1
	{     EDID_VIDEO_NONE,    50,    0,     2560,   1080,        0,      0},    //  89  2560x1080p              50Hz            64:27       1:1
	{     EDID_VIDEO_NONE,    60,    0,     2560,   1080,        0,      0},    //  90  2560x1080p              60Hz            64:27       1:1
	{     EDID_VIDEO_NONE,   100,    0,     2560,   1080,        0,      0},    //  91  2560x1080p             100Hz            64:27       1:1
	{     EDID_VIDEO_NONE,   120,    0,     2560,   1080,        0,      0},    //  92  2560x1080p             120Hz            64:27       1:1
	{     EDID_VIDEO_NONE,    24,    0,     3840,   2160,        0,      0},    //  93  3840x2160p              24Hz            16:9        1:1
	{     EDID_VIDEO_NONE,    25,    0,     3840,   2160,        0,      0},    //  94  3840x2160p              25Hz            16:9        1:1
	{     EDID_VIDEO_NONE,    30,    0,     3840,   2160,        0,      0},    //  95  3840x2160p              30Hz            16:9        1:1
	{     EDID_VIDEO_NONE,    50,    0,     3840,   2160,        0,      0},    //  96  3840x2160p              50Hz            16:9        1:1
	{     EDID_VIDEO_NONE,    60,    0,     3840,   2160,        0,      0},    //  97  3840x2160p              60Hz            16:9        1:1
	{     EDID_VIDEO_NONE,    24,    0,     4096,   2160,        0,      0},    //  98  4096x2160p              24Hz           256:135      1:1
	{     EDID_VIDEO_NONE,    25,    0,     4096,   2160,        0,      0},    //  99  4096x2160p              25Hz           256:135      1:1
	{     EDID_VIDEO_NONE,    30,    0,     4096,   2160,        0,      0},    // 100  4096x2160p              30Hz           256:135      1:1
	{     EDID_VIDEO_NONE,    50,    0,     4096,   2160,        0,      0},    // 101  4096x2160p              50Hz           256:135      1:1
	{     EDID_VIDEO_NONE,    60,    0,     4096,   2160,        0,      0},    // 102  4096x2160p              60Hz           256:135      1:1
	{     EDID_VIDEO_NONE,    24,    0,     3840,   2160,        0,      0},    // 103  3840x2160p              24Hz            64:27       4:3
	{     EDID_VIDEO_NONE,    25,    0,     3840,   2160,        0,      0},    // 104  3840x2160p              25Hz            64:27       4:3
	{     EDID_VIDEO_NONE,    30,    0,     3840,   2160,        0,      0},    // 105  3840x2160p              30Hz            64:27       4:3
	{     EDID_VIDEO_NONE,    50,    0,     3840,   2160,        0,      0},    // 106  3840x2160p              50Hz            64:27       4:3
	{     EDID_VIDEO_NONE,    60,    0,     3840,   2160,        0,      0},    // 107  3840x2160p              60Hz            64:27       4:3
};

typedef struct _tHDMI_VIC
{
	unsigned long	PixelClock;             // 0000h Pixel clock, in MHz.
	unsigned char	Frequency;              // 0004h Refresh rate, in Hz.
	unsigned short	HorAddrTime;
	unsigned short	HorBlank;
	unsigned short	HorFront;
	unsigned short	HorSyncTime;
	unsigned short	VerAddrTime;
	unsigned short	VerBlank;
	unsigned short 	VerFront;
	unsigned short	VerSyncTime;
}HDMI_VIC, *PHDMI_VIC;

static HDMI_VIC	g_HdmiVicTable[5] = {
	{0,		 0,     0,	     0,	       0,     0,       0,       0,    0,    0},
	{297,   30,	 3840,     560,      176,    88,    2160,      90,    8,   10},	//0x01 3840x2160 30Hz  297MHz
	{297,   25,  3840,    1440,     1056,    88,    2160,      90,    8,   10},	//0x02 3840x2160 25Hz  297MHz
	{297,   24,  3840,    1660,     1276,    88,    2160,      90,    8,   10},	//0x03 3840x2160 24Hz  297MHz
	{297,   24,  4096,    1404,     1020,    88,    2160,      90,    8,   10},	//0x04 4096x2160 24Hz  297MHz
};

int EDID_ValidateBlock1Buffer
(
    PUCHAR              pEDIDBuffer,
    PUCHAR              pExtensionTag,
    PUCHAR              pExtensionVersion
)
{
	PUCHAR  p;
	UCHAR   ucData;
	UCHAR   EDIDMajorVersion;
	UCHAR   EDIDMinorVersion;
	USHORT  i;

	if (!pEDIDBuffer || !pExtensionTag || !pExtensionVersion) {
		return -1;
	}

	*pExtensionTag = 0xBD;
	*pExtensionVersion = 0xBD;

	ucData = 0;
	p = pEDIDBuffer;

	for (i=0; i<EDID_BLOCK_LENGTH; i++) {
		ucData += *p++;
	}

	if (ucData) {
		DebugPrint ("  invalid EDID checksum (0x%X) failed. data is corrupt.\n", ucData);
		return -1;
	}

	*pExtensionTag = pEDIDBuffer[EDID_REG_EXTENSION_TAG];
	*pExtensionVersion = pEDIDBuffer[EDID_REG_EXTENSION_REVISION];

	if (pEDIDBuffer[EDID_REG_EXTENSION_TAG] != EDID_EXTENSION_TAG) {
		DebugPrint (" invalid EDID extension tag (0x%02X).\n", pEDIDBuffer[EDID_REG_EXTENSION_TAG]);
		return -1;
	}

    if (pEDIDBuffer[EDID_REG_EXTENSION_REVISION] != EDID_EXTENSION_REVISION) {
       DebugPrint (" invalid EDID extension revision (0x%02X).\n", pEDIDBuffer[EDID_REG_EXTENSION_REVISION]);
        return -1;
    }

	return 0;

}

/*****************************************************************************
 * EDID_ParseDetailDescriptor
 *****************************************************************************
 * Description:
 *
 * Variables:
 *
 * Return:
 *
 */
int EDID_ParseDetailDescriptor
(
    ULONG               Index,
    PUCHAR              pDescriptorBuffer,
    PUCHAR              pMonitorName,
	PUCHAR				bRun4K30
)
{

    PUCHAR pEDIDBuf = pDescriptorBuffer;
    PUCHAR  p;
    UINT8 ucData;
    UINT16 j;
    UINT16 usData;
    UINT16 usHorActPix;
    UINT16 usVerActPix;
    UINT16 usRefRate;
    UINT32 ulMinHorFreq;
    UINT32 ulMaxHorFreq;
    UINT32 ulMinVerFreq;
    UINT32 ulMaxVerFreq;
	UINT32 ulMaxPixelClk;


    usData = *((PUSHORT)pEDIDBuf);
DebugPrint("enter %s , usData = %x\n", __func__, usData);
    if (usData == 0x0000)
    {  // Monitor descriptor
        ucData = *(pEDIDBuf + 3);
DebugPrint("enter %s , ucData = %x\n", __func__, ucData);
        switch (ucData)
        {
        case 0x0F: // Defined by manufacturer
			DebugPrint ("    [%ld] Manufacturer Defined Descriptor (0x0F)\n", Index);
            break;

        case 0xF9: // Currently Undefined
            DebugPrint ("    [%ld] Currently Undefined Descriptor (0xF9)\n", Index);
            break;

        case 0xFA: // Standard Timing
			DebugPrint ("    [%ld] Standard Timing Descriptor (0xFA):\n", Index);

            for (j=0; j<6; j++)
            {
                usData = *(PUSHORT)pEDIDBuf;

                if (usData == 0x0101)
                {
                    //DebugPrint ("      [%d] 0x%04X Unused Startard Timing.\n", j, usData);
                }
                else
                {
                    //usHorActPix = (*pEDIDBuf + 31) * 8;
                    //pEDIDBuf++;
                    //usRefRate = (*pEDIDBuf & 0x3F) + 60;
                    //ucData = ((*pEDIDBuf) >> 6);
                    //pEDIDBuf++;

                    // Horizontal Active pixels
                    usHorActPix = ((usData & 0xFF) + 0x1F) * 8;

                    // Aspect Ratio & Refresh Rate
                    ucData = (UCHAR)(usData >> 8);
                    usRefRate = /*(USHORT)*/(ucData & 0x3F) + 60;
                    ucData >>= 6;

                    switch (ucData)
                    {
                    case 0: // Aspect Ratio = 16:10
                        //usVerActPix = (usHorActPix * 10) / 16;
                        usVerActPix = (usHorActPix * 5) >> 3;
                        break;

                    case 1: // Aspect Ratio =  4:3
                        //usVerActPix = (usHorActPix * 3) / 4;
                        usVerActPix = (usHorActPix * 3) >> 2;
                        break;

                    case 2: // Aspect Ratio =  5:4
                        usVerActPix = (usHorActPix * 4) / 5;
                        break;

                    case 3: // Aspect Ratio = 16:9
                        //usVerActPix = (usHorActPix * 9) / 16;
                        usVerActPix = (usHorActPix * 9) >> 4;
                        break;

                    default:
                        usVerActPix = 0;
                        break;
                    }

                    if (usVerActPix && usHorActPix && usVerActPix && usRefRate)
                    {
                        DebugPrint ("      [%ld] 0x%04X %dx%d %dHz\n", Index, usData, usHorActPix, usVerActPix, usRefRate);
						if(usHorActPix >= 3840 && usVerActPix >= 2160) *bRun4K30 = 1;
                        //TraceEvents(TRACE_LEVEL_INFORMATION, TRACE_EDID, "Ext Detail: %dx%d %dHz ", usHorActPix, usVerActPix, usRefRate);
						//MCT_ValidModeEDID (pDispObj, usHorActPix, usVerActPix, usRefRate);

                    }
                    else
                    {
                        //DebugPrint ("      [%d] 0x%04X %dx%d %dHz\n", Index, usData, usHorActPix, usVerActPix, usRefRate);
                    }
                }

                pEDIDBuf += 2;
            }//End for
            break;

        case 0xFB: // Colour Pointer
            DebugPrint ("	[%ld] Colour Pointer Descriptor (0xFB)\n", Index);
            break;

        case 0xFC: // Monitor Name
            break;

        case 0xFD: // Monitor Range Limits
            MctPrint ("    [%ld] Monitor Range Limits Descriptor (0xFD):\n", Index);
            ucData = *(pEDIDBuf+ 5);
            ulMinVerFreq = (ULONG) ucData;
            ulMinVerFreq = ulMinVerFreq*10;

            ucData = *(pEDIDBuf+ 6);
            ulMaxVerFreq = (ULONG) ucData;
            ulMaxVerFreq = ulMaxVerFreq*10;

            ucData = *(pEDIDBuf+ 7);
            ulMinHorFreq = (ULONG) ucData;
            ulMinHorFreq = ulMinHorFreq*10;

            ucData = *(pEDIDBuf+ 8);
            ulMaxHorFreq = (ULONG) ucData;
            ulMaxHorFreq = ulMaxHorFreq*10;
			
			ucData = *(pEDIDBuf+ 9);
            ulMaxPixelClk = (ULONG) ucData;
            ulMaxPixelClk = ulMaxPixelClk*10;

			DebugPrint ("        Vertical Frequency: %d ~ %d Hz\n", ulMinVerFreq, ulMaxVerFreq);
			DebugPrint ("        Horizontal Frequency: %d ~ %d Hz\n", ulMinHorFreq, ulMaxHorFreq);
			DebugPrint ("        ulMaxPixelClk: %d MHz\n", ulMaxPixelClk);
			if(ulMaxPixelClk >= 300) *bRun4K30 = 1;
			break;

        case 0xFE: // ASCII String
           DebugPrint("    [%ld] ASCII String Descriptor (0xFE)\n", Index);
            break;

        case 0xFF: // Monitor Serial Number
            DebugPrint("    [%ld] Monitor Serial Number Descriptor (0xFF)\n", Index);
            break;

        default:
            DebugPrint("    [%ld] Unknown Descriptor (0x%02X)\n", Index, ucData);
			break;
        } //switch
    }
    else
    {   // detailed timing format
        UINT32 u32Data;
        UINT32 u32PixelClock;
        UINT16 u16HorActive;
        UINT16 u16VerActive;
        UINT16 u16HorBlanking;
        UINT16 u16VerBlanking;
        UINT16 u16HorSyncOffset;
        UINT16 u16VerSyncOffset;
        UINT16 u16HorSyncWidth;
        UINT16 u16VerSyncWidth;
        UINT8 u8HorBorder;
        UINT8 u8VerBorder;
        BOOLEAN bInterlacedMode;
        BOOLEAN bHorSyncPositive;
        BOOLEAN bVerSyncPositive;

        DebugPrint("    [%ld] Detailed Timing Descriptor\n", Index);
		//TraceEvents(TRACE_LEVEL_INFORMATION, TRACE_DEVICE, "    [%d] Detailed Timing Descriptor\n", Index);

        ucData = *(pEDIDBuf + 0x11);

        if (ucData & BIT_7)
        {
            bInterlacedMode = TRUE;
           //DebugPrint ("        Interlaced Mode.\n");
        }
        else
        {
            bInterlacedMode = FALSE;
            //DebugPrint("        Non-interlaced Mode.\n");
        }

        if (!(ucData & BIT_5) && !(ucData & BIT_6))
        {
           // DebugPrint("        Normal Display, No Stereo.\n");
        }
        else
        {   // Refer to VESA E-EDID Release A, Revision 1, table 3.17
            //DebugPrint("        Enhance Display.\n");
        }

        if (ucData & BIT_3)
        {
            if (ucData & BIT_4)
            {
               // DebugPrint("        Digital Separate.\n");
            }
            else
            {
                //DebugPrint("        Bipolar Analog Composite.\n");
            }
        }
        else
        {
            if (ucData & BIT_4)
            {
                //DebugPrint("        Digital Composite.\n");
            }
            else
            {
               // DebugPrint("        Analog Composite.\n");
            }
        }

        bVerSyncPositive = (ucData & BIT_2) ? TRUE : FALSE;
        bHorSyncPositive = (ucData & BIT_1) ? TRUE : FALSE;

        //DebugPrint("        Vertical Sync is %s.\n", bVerSyncPositive ? "positive" : "negative");
        //DebugPrint("        Horizontal Sync is %s.\n", bHorSyncPositive ? "positive" : "negative");

        // Horizontal Active and Horizontal Blanking
        //Pixel clock
        usData = (*(PUSHORT)pEDIDBuf);
        u32PixelClock = ((ULONG) usData) * 10000;

        ucData = *(pEDIDBuf + 0x04);
        u16HorActive = ucData & 0xF0;
        u16HorActive <<= 4;
        u16HorBlanking = ucData & 0x0F;
        u16HorBlanking <<= 8;
        ucData = *(pEDIDBuf + 0x02);
        u16HorActive += ucData;
        ucData = *(pEDIDBuf + 0x03);
        u16HorBlanking += ucData;

        // Vertical Active and Vertical Blanking
        ucData = *(pEDIDBuf + 0x07);
        u16VerActive = ucData & 0xF0;
        u16VerActive <<= 4;
        u16VerBlanking = ucData & 0x0F;
        u16VerBlanking <<= 8;
        ucData =*(pEDIDBuf + 0x05);
        u16VerActive += ucData;
        ucData =*(pEDIDBuf + 0x06);
        u16VerBlanking += ucData;

        ucData = *(pEDIDBuf + 11) & 0xC0;
        u16HorSyncOffset = *(pEDIDBuf + 8);
        u16HorSyncOffset += (USHORT)ucData << 2;
        ucData = *(pEDIDBuf + 11) & 0x30;
        u16HorSyncWidth = (USHORT)*(pEDIDBuf + 9) + ((USHORT)ucData<<4);

        ucData = *(pEDIDBuf + 11) & 0x0C;
        u16VerSyncOffset = (USHORT)((*(pEDIDBuf +10)&0xF0)>>4) + ((USHORT)ucData<<2);

        ucData = *(pEDIDBuf + 11) & 0x03;
        u16VerSyncWidth = (USHORT)(*(pEDIDBuf +10)&0x0F) + ((USHORT)ucData<<4);

        u8HorBorder = *(pEDIDBuf + 15);
        u8VerBorder = *(pEDIDBuf + 16);

		//usRefRate   = (USHORT) (u32PixelClock / (((ULONG)u16HorActive + u16HorBlanking)*((ULONG)u16VerActive + u16VerBlanking)));
		u32Data = ((ULONG)u16HorActive + u16HorBlanking) * ((ULONG)u16VerActive + u16VerBlanking);

		//avoid divide by 0
		if (u32Data)
		{
			usRefRate = (USHORT) (((u32PixelClock << 1) + u32Data) / (u32Data << 1)); // rounding off

			DebugPrint("Extra:        Prefer Timing: %dx%dx%dHz\n", u16HorActive, u16VerActive, usRefRate);
			DebugPrint("Extra:        Pixel Clock=%d Hz\n", u32PixelClock);
			DebugPrint("Extra:        Horizontal Blanking=%d SyncOffset=%d SyncWidth=%d Border=%d\n", u16HorBlanking, u16HorSyncOffset, u16HorSyncWidth, u8HorBorder);
			DebugPrint("Extra:        Vertical   Blanking=%d SyncOffset=%d SyncWidth=%d Border=%d\n", u16VerBlanking, u16VerSyncOffset, u16VerSyncWidth, u8VerBorder);
			
			if(u16HorActive >= 3840 && u16VerActive >= 2160) *bRun4K30 = 1;

		}
		else
		{
			DebugPrint("        Invalid EDID timing\n");
		}
    }

	return 0;
}

/*****************************************************************************
* EDID_ParseVendorSpecificBlock
*****************************************************************************
* Description:
*
* Variables:
*
* Return:
*
*/
int EDID_ParseVendorSpecificBlock
(
	PUCHAR  pVendorBlock,
	PUCHAR  bRun4K30
)
{
	PUCHAR	p;
	UCHAR	VideoCapsStart;
	UCHAR	HDMI_VIC_len;
	UCHAR	i;

#define Latency_Fields_Present	0x80
#define I_Latency_Fields_Present 0x40
#define HDMI_Video_Present		0x20

	DebugPrint("Max TMDS Clock : %d MHz HDMI Caps:0x%x\n", pVendorBlock[6] * 5, pVendorBlock[7]);
	if (pVendorBlock[7] & HDMI_Video_Present)
	{
		VideoCapsStart = 8;
		if (pVendorBlock[7] & Latency_Fields_Present)
			VideoCapsStart += 2;
		if (pVendorBlock[7] & I_Latency_Fields_Present)
			VideoCapsStart += 2;
		//DebugPrint("3D Present: %d 3D Multi Present:%d\n",
		//	(pVendorBlock[VideoCapsStart] & 0x80) >> 7, (pVendorBlock[VideoCapsStart] & 0x40) >> 6);
		HDMI_VIC_len = (pVendorBlock[VideoCapsStart + 1] >> 5) & 0x07;
		MctPrint(" HDMI VIC length :%d\n", HDMI_VIC_len);
		VideoCapsStart += 2;
		for (i = 0; i < HDMI_VIC_len; i++)
		{
MctPrint(" VideoCapsStart :%d\n", VideoCapsStart);
			UCHAR VIC = pVendorBlock[VideoCapsStart + i];
MctPrint(" VIC :%d\n", VIC);
            if(VIC <= 5) {
                DebugPrint("VIC[%d]: %dx%d %dHz\n",
                    VIC, g_HdmiVicTable[VIC].HorAddrTime, g_HdmiVicTable[VIC].VerAddrTime, g_HdmiVicTable[VIC].Frequency);
					
				if(	g_HdmiVicTable[VIC].HorAddrTime >=3840 && g_HdmiVicTable[VIC].VerAddrTime >= 2160) *bRun4K30 = 1;
					
            }else
                DebugPrint("VIC value is over limit(5), cannot get value from g_HdmiVicTable");
		}
	}
	return 0;
}


/*****************************************************************************
 * EDID_ParseCEAExtensionBlockInformation
 *****************************************************************************
 * Description:
 *
 * Variables:
 *
 * Return:
 *
 */
 int EDID_ParseCEAExtensionBlockInformation
(
	PUCHAR  	pEDIDBuf,
    UCHAR		IndexOfExtension,
	PUCHAR		bRun4K30
)
{
    int ret = 0;

    PUCHAR  pjEDIDBuf;
    UCHAR   ExtensionTag = 0x00;
    UCHAR   ExtensionVersion = 0x00;
    UCHAR   i, j;
    UCHAR   Index;
    UCHAR   IndexMax;
    UCHAR   ucData;
    UCHAR   LongDescriptorOffset;
    UCHAR   DescriptorType;
    UCHAR   DescriptorLength;
    USHORT  usData;
    PUCHAR  p;
	char    monitor_name[128];


	memset(monitor_name, 0, 128);


    DebugPrint(" parsing extension %d\n", IndexOfExtension);

//pEDIDBuf = EDID_LG_4K;
    pjEDIDBuf = pEDIDBuf + ((IndexOfExtension + 1) * EDID_BLOCK_LENGTH);
    ret = EDID_ValidateBlock1Buffer (pjEDIDBuf, &ExtensionTag, &ExtensionVersion);

    if (!ret)
    {
        if ((ExtensionTag == 0xBD) && (ExtensionVersion == 0xBD))
        {   // checksum error
        }
        else
        if (ExtensionTag == EDID_EXTENSION_TAG)
        {
            if (ExtensionVersion > EDID_EXTENSION_REVISION)
            {   // unknown extension revision
               return -1;
            }
        }
        else
        {   // unknown extension tag
            return -1;
        }
    }

    DebugPrint(" EDID revision: %d.%d\n", pjEDIDBuf[EDID_REG_EXTENSION_TAG], pjEDIDBuf[EDID_REG_EXTENSION_REVISION]);

    LongDescriptorOffset = pjEDIDBuf[EDID_REG_LONG_DESCRIPTOR_OFFSET];    // block offset where long descriptors start

    ucData = pjEDIDBuf[EDID_REG_MISC_SUPPORT];

    //DebugPrint("  Miscellaneous support (0x%02X):\n", ucData);

    if (ucData & 0x0F)
    {
       //DebugPrint("    Total number of native DTDs=%d\n", ucData & 0x0F);
    }
    if (ucData & 0x10)
    {
       // DebugPrint("    YCbCr_4_4_4 supported.\n");
    }
    if (ucData & 0x20)
    {
        //DebugPrint( "    YCbCr_4_2_2 supported.\n");
    }
    if (ucData & 0x40)
    {
        //MctPrint ("    Basic Audio supported.\n");
    }
    if (ucData & 0x80)
    {
       // MctPrint ("    Under Scan supported.\n");
    }

    Index = EDID_REG_DATA_START;
    IndexMax = LongDescriptorOffset;

    //MctPrint ("  Short Descriptors (0x04 ~ 0x%02X):\n", IndexMax);

    while (Index < IndexMax)
    {
        ucData = pjEDIDBuf[Index++];

        DescriptorType = (ucData >> 5) & 0x07;
        DescriptorLength = ucData & 0x1F;

        if ((Index + DescriptorLength) > LongDescriptorOffset)
        {
           // MctPrint ("Descriptor Overflow\n");
            break;
        }

        switch (DescriptorType)
        {
        case EDID_DATA_BLOCK_AUDIO:
            MctPrint ("    [0x%02X] Audio Data Block (%d). length=%d.\n", ucData, DescriptorType, DescriptorLength);
			//yulw
			if (DescriptorLength == 3)
			{
				MctPrint(" Audio Format code: %d\n", pjEDIDBuf[Index] >> 3);
				MctPrint(" Channels: %d\n", (pjEDIDBuf[Index] & 0x07) + 1);
				MctPrint(" Sampling frequencies: 0x%x\n", pjEDIDBuf[Index + 1]);
				MctPrint(" BitRate: 0x%x\n", pjEDIDBuf[Index + 2]);

			}
            break;

        case EDID_DATA_BLOCK_VIDEO:
			MctPrint ("    [0x%02X] Video Data Block (%d). length=%d.\n", ucData, DescriptorType, DescriptorLength);

            p = pjEDIDBuf + Index;

            for (i=0; i<DescriptorLength; i++)
            {
                j = *p++;
				j &= 0x7F;
				MctPrint ("      0x%02X:%dx%d%c%d\n",
                    j, g_VideoDataFormat[j].Horizontal, g_VideoDataFormat[j].Vertical, (g_VideoDataFormat[j].Flags & EDID_VIDEO_INTERLACE) ? 'i' : 'p', g_VideoDataFormat[j].Rate);
				
				if(g_VideoDataFormat[j].Horizontal >= 3840 || g_VideoDataFormat[j].Vertical >= 2160) *bRun4K30  = 1;

            }
            break;

        case EDID_DATA_BLOCK_VENDOR_SPECIFIC:
			MctPrint ("    [0x%02X] Vendor Specific Data Block (%d). length=%d.\n", ucData, DescriptorType, DescriptorLength);

            // check if the sink is HDMI compatible or not.
            if ((pjEDIDBuf[Index    ] == 0x03) &&
                (pjEDIDBuf[Index + 1] == 0x0C) &&
                (pjEDIDBuf[Index + 2] == 0x00))
            {
                MctPrint ("      HDMI 1.x sink.\n");
				ret = EDID_ParseVendorSpecificBlock(&pjEDIDBuf[Index], bRun4K30);
            }
			else
			if ((pjEDIDBuf[Index] == 0xD8) &&
				(pjEDIDBuf[Index + 1] == 0x5D) &&
				(pjEDIDBuf[Index + 2] == 0xC4))
			{
				MctPrint("      HDMI 2.0 sink.\n");
				//ret = EDID_ParseVendorSpecificBlock(&pjEDIDBuf[Index]);
			}
			else
            {
                //MctPrint ("      DVI sink.\n");

            }
			if(ret < 0) return -1;

            break;

        case EDID_DATA_BLOCK_SPEAKER_ALLOCATION:
           // MctPrint ("    [0x%02X] Speaker Allocation Data Block (%d). length=%d.\n", ucData, DescriptorType, DescriptorLength);
            break;

        case EDID_DATA_BLOCK_VESA_DTC:
            //MctPrint ("    [0x%02X] VESA DTC Data Block (%d). length=%d.\n", ucData, DescriptorType, DescriptorLengt));
            break;

        case EDID_DATA_BLOCK_USE_EXTENDED_TAG:
           // MctPrint ("    [0x%02X] Use Extended Tag Data Block (%d). length=%d.\n", ucData, DescriptorType, DescriptorLength);
            break;

        default:
            //MctPrint ("    [0x%02X] Unknown Data Block (%d). length=%d.\n", ucData, DescriptorType, DescriptorLength);
            break;
        }

        Index += DescriptorLength;
    }

    if (Index < IndexMax)
    {   // something wrong
        MctPrint ("    something wrong occurred when short descriptors are parsing, 0x%02X < 0x%02X\n", Index, IndexMax);
        return -1;
    }

    Index = LongDescriptorOffset;
    IndexMax = EDID_REG_CHECKSUM - DETAILED_DESCRIPTOR_LENGTH;

    MctPrint ("  Long Descriptors (0x%02X ~ 0x%02X):\n", LongDescriptorOffset, IndexMax);

	/*
	if (LongDescriptorOffset > IndexMax)
	{
		p = pjEDIDBuf + Index;
		MctPrint("[%x] ~[7F]:", Index);
		for (i = Index; i <= 0x7F; i++)
			MctPrint(" %02x", *p++);

		MctPrint("\n");
	}
	*/
	i = 0;
    while (Index < IndexMax)
    {
        p = pjEDIDBuf + Index;

        EDID_ParseDetailDescriptor (
            i++,
            p,
            monitor_name,
			bRun4K30
		);

        Index += DETAILED_DESCRIPTOR_LENGTH;
    }

    if (Index < IndexMax)
    {   // something wrong
       MctPrint ("    something wrong occurred when long descriptors are parsing, 0x%02X < 0x%02X\n", Index, IndexMax);
	   return -1;
    }

    MctPrint("*****************Leave %s ******************\n", __func__);
    return 0;
}
