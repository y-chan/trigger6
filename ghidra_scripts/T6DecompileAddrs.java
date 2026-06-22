// Decompiles functions at addresses passed as script arguments into ghidra_out.
// @category T6

import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileOptions;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;

import java.io.File;
import java.io.FileWriter;

public class T6DecompileAddrs extends GhidraScript {
    @Override
    protected void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length == 0) {
            println("usage: T6DecompileAddrs.java <addr>...");
            return;
        }

        File outDir = new File("ghidra_out");
        outDir.mkdirs();

        DecompInterface decompiler = new DecompInterface();
        DecompileOptions options = new DecompileOptions();
        options.grabFromProgram(currentProgram);
        decompiler.setOptions(options);
        decompiler.openProgram(currentProgram);

        for (String arg : args) {
            Address addr = currentProgram.getAddressFactory().getDefaultAddressSpace().getAddress(arg);
            Function function = getFunctionContaining(addr);
            if (function == null) {
                println("no function at " + arg);
                continue;
            }
            DecompileResults results = decompiler.decompileFunction(function, 90, monitor);
            String name = function.getName();
            String filename = String.format("%s_%s.c", function.getEntryPoint(), name).replaceAll("[^A-Za-z0-9_.-]", "_");
            File out = new File(outDir, filename);
            try (FileWriter writer = new FileWriter(out)) {
                writer.write("// function=" + name + "\n");
                writer.write("// entry=" + function.getEntryPoint() + "\n");
                writer.write("// status=" + results.getErrorMessage() + "\n\n");
                if (results.decompileCompleted()) {
                    writer.write(results.getDecompiledFunction().getC());
                } else {
                    writer.write("/* decompile failed */\n");
                }
            }
            println("wrote " + out.getPath());
        }

        decompiler.dispose();
    }
}
