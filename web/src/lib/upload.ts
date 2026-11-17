import { fromJson, type JsonValue } from "@bufbuild/protobuf";
import { UploadedFirmwareSchema, type UploadedFirmware } from "../gen/dphone/v1/firmware_pb";
import { csrfToken } from "./session";

/// Streams a file to one of the upload endpoints, reporting progress.  The
/// reply is an UploadedFirmware message in JSON; an app upload fills in
/// only the upload ID and the size.
export function upload(path: "/upload/firmware" | "/upload/app", file: Blob, onProgress?: (fraction: number) => void): Promise<UploadedFirmware> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", path);
    xhr.setRequestHeader("X-Csrf-Token", csrfToken());
    xhr.setRequestHeader("Content-Type", "application/octet-stream");
    xhr.upload.onprogress = (e) => e.lengthComputable && onProgress?.(e.loaded / e.total);
    xhr.onerror = () => reject(new Error("Upload failed."));
    xhr.onload = () => {
      let body: JsonValue = null;
      try {
        body = JSON.parse(xhr.responseText) as JsonValue;
      } catch {
        // Not JSON: fall through to the status text.
      }
      if (xhr.status === 200 && body !== null) {
        try {
          const up = fromJson(UploadedFirmwareSchema, body, { ignoreUnknownFields: true });
          if (up.uploadId) return resolve(up);
        } catch {
          // Not the reply expected: fall through.
        }
      }
      const message = body !== null && typeof body === "object" && !Array.isArray(body) && typeof body.message === "string" ? body.message : "";
      reject(new Error(message || `Upload failed (${xhr.status} ${xhr.statusText})`));
    };
    xhr.send(file);
  });
}
