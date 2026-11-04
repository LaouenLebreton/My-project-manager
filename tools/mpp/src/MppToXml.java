import java.io.File;
import java.io.FileOutputStream;
import java.io.OutputStream;

import org.mpxj.MPXJException;
import org.mpxj.ProjectFile;
import org.mpxj.mpp.MPPReader;
import org.mpxj.mspdi.MSPDIWriter;

/**
 * Super PM's .mpp converter: reads a Microsoft Project file with MPXJ and writes it as
 * Microsoft Project XML, which Super PM imports with its own checks and import report.
 *
 * Usage: MppToXml input.mpp output.xml
 *
 * Exit codes: 0 written; 2 the file cannot be read as a Microsoft Project file; 3 it is
 * password-protected; 1 anything else. On failure, one line in plain words goes to stderr.
 */
public final class MppToXml {
    public static void main(String[] args) {
        if (args.length != 2) {
            fail(1, "usage: MppToXml input.mpp output.xml");
        }
        File in = new File(args[0]);
        File out = new File(args[1]);
        if (!in.isFile()) {
            fail(2, "the file could not be found");
        }
        ProjectFile project;
        try {
            MPPReader reader = new MPPReader();
            // Bar styles and view layouts: Super PM has its own.
            reader.setReadPresentationData(false);
            project = reader.read(in);
        } catch (MPXJException e) {
            String message = String.valueOf(e.getMessage()).toLowerCase();
            if (message.contains("password")) {
                fail(3, "this file is password-protected; open it in Microsoft Project, remove the password, and try again");
            }
            fail(2, "this is not a Microsoft Project file Super PM can read (" + e.getMessage() + ")");
            return;
        } catch (RuntimeException e) {
            fail(2, "this is not a Microsoft Project file Super PM can read (" + e + ")");
            return;
        }
        try (OutputStream stream = new FileOutputStream(out)) {
            MSPDIWriter writer = new MSPDIWriter();
            // Timephased data (work per day) is not imported; leave it out.
            writer.setWriteTimephasedData(false);
            writer.write(project, stream);
        } catch (Exception e) {
            fail(1, "the converted project could not be written (" + e + ")");
        }
        System.exit(0);
    }

    private static void fail(int code, String message) {
        System.err.println(message);
        System.exit(code);
    }
}
