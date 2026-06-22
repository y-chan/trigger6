// Lists interesting T6 driver strings and references to them.
// @category T6

import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.data.StringDataInstance;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.Listing;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.ReferenceIterator;

import java.util.ArrayList;
import java.util.List;

public class T6StringXrefs extends GhidraScript {
    private static final String[] NEEDLES = new String[] {
        "mct_jpeg_videotoolbox_encode",
        "jpeg_videotoolbox.cpp",
        "t6_metaljpeg_compress_texture_async",
        "t6_compress_yuv420",
        "t6_compress_yuv420_gpurle",
        "t6_compress_yuv420_gpu_dctquantrlehuff_whole_image",
        "jpegencode_dctquant_420",
        "jpegencode_dctquant_420_2mb_parallel_sbb",
        "jpegencode_dctquant_444_4b_parallel_sbb",
        "jpegencode_dctquant_rle_420",
        "jpegencode_rlehuffman_420",
        "jpegencode_rlehuffman_444",
        "t6_upload_uncompressed_yuv420",
        "t6_upload_uncompressed_yuv444",
        "t6_submit_frame_surface_with_compressed_dirty_rects",
        "t6_submit_frame_surface_whole_screen_compressed",
        "t6_submit_frame_surface_whole_screen_compressed_vt_jpeg",
        "MCTT6Device JPEG Encoder",
        "vt_yuv420_encoder",
        "not using JPEG encoding due to blacklisted firmware"
    };

    @Override
    protected void run() throws Exception {
        println("program=" + currentProgram.getName());
        println("image_base=" + currentProgram.getImageBase());
        println("");

        List<Data> hits = new ArrayList<>();
        Listing listing = currentProgram.getListing();
        for (Data data : listing.getDefinedData(true)) {
            if (monitor.isCancelled()) {
                break;
            }
            String text = getStringValue(data);
            if (text == null) {
                continue;
            }
            for (String needle : NEEDLES) {
                if (text.contains(needle)) {
                    hits.add(data);
                    break;
                }
            }
        }

        for (Data data : hits) {
            Address addr = data.getAddress();
            String text = getStringValue(data);
            MemoryBlock block = currentProgram.getMemory().getBlock(addr);
            println("STRING " + addr + " block=" + (block == null ? "?" : block.getName()) + " len=" + text.length());
            println("  " + text.replace("\n", "\\n"));

            ReferenceIterator refs = currentProgram.getReferenceManager().getReferencesTo(addr);
            int count = 0;
            while (refs.hasNext()) {
                Reference ref = refs.next();
                count++;
                Address from = ref.getFromAddress();
                Function fn = listing.getFunctionContaining(from);
                println("  REF from=" + from + " type=" + ref.getReferenceType() +
                    " fn=" + (fn == null ? "?" : fn.getName()) +
                    " entry=" + (fn == null ? "?" : fn.getEntryPoint()));
            }
            if (count == 0) {
                println("  REF none");
            }
            println("");
        }
        println("hits=" + hits.size());
    }

    private String getStringValue(Data data) {
        Object value = data.getValue();
        if (value instanceof String) {
            return (String)value;
        }
        StringDataInstance sdi = StringDataInstance.getStringDataInstance(data);
        if (sdi == null) {
            return null;
        }
        String valueString = sdi.getStringValue();
        if (valueString == null || valueString.isEmpty()) {
            return null;
        }
        return valueString;
    }
}
