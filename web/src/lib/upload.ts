import { csrfToken } from "./session";

export type Uploaded = {
  uploadId: string;
  size: string | number;
  version?: string;
  model?: string;
  isStock?: boolean;
  modelMatches?: boolean;
};

/// Streams a file to one of the upload endpoints, reporting progress.
export function upload(path: "/upload/firmware" | "/upload/app", file: Blob, onProgress?: (fraction: number) => void): Promise<Uploaded> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", path);
    xhr.setRequestHeader("X-Csrf-Token", csrfToken());
    xhr.setRequestHeader("Content-Type", "application/octet-stream");
    xhr.upload.onprogress = (e) => e.lengthComputable && onProgress?.(e.loaded / e.total);
    xhr.onerror = () => reject(new Error("The upload was interrupted."));
    xhr.onload = () => {
      let body: { message?: string } & Partial<Uploaded> = {};
      try {
        body = JSON.parse(xhr.responseText);
      } catch {
        // Not JSON: fall through to the status text.
      }
      if (xhr.status === 200 && body.uploadId) resolve(body as Uploaded);
      else reject(new Error(body.message || `Upload failed (${xhr.status} ${xhr.statusText})`));
    };
    xhr.send(file);
  });
}
