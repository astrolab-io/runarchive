use crate::deflate::ProgressObserver;

pub struct IndicatifProgress {
    pub pb_download: Option<indicatif::ProgressBar>,
    pub pb_decompress: Option<indicatif::ProgressBar>,
    pub mp: Option<indicatif::MultiProgress>,
}

impl ProgressObserver for IndicatifProgress {
    fn update_download(&self, bytes: u64) {
        if let Some(ref pb) = self.pb_download {
            pb.inc(bytes);
        }
    }
    fn update_decompress(&self, bytes: u64) {
        if let Some(ref pb) = self.pb_decompress {
            pb.inc(bytes);
        }
    }
    fn set_download_position(&self, bytes: u64) {
        if let Some(ref pb) = self.pb_download {
            pb.set_position(bytes);
        }
    }
    fn set_decompress_position(&self, bytes: u64) {
        if let Some(ref pb) = self.pb_decompress {
            pb.set_position(bytes);
        }
    }
    fn println(&self, msg: String) {
        if let Some(ref mp) = self.mp {
            mp.println(msg).unwrap_or(());
        } else {
            println!("{}", msg);
        }
    }
    fn finish(&self) {
        if let Some(ref pb) = self.pb_download {
            pb.finish_with_message("Done");
        }
        if let Some(ref pb) = self.pb_decompress {
            pb.finish_with_message("Done");
        }
    }
}
