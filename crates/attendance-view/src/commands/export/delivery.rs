use std::future::Future;

use crate::{Error, attendance_export::IdentityMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, poise::ChoiceParameter)]
pub(super) enum ExportFormat {
    #[name = "all"]
    All,
    #[name = "csv"]
    Csv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FileKind {
    CsvWithDiscord,
    CsvRealName,
    PdfWithDiscord,
    PdfRealName,
}

impl FileKind {
    pub fn is_pdf(self) -> bool {
        matches!(self, Self::PdfWithDiscord | Self::PdfRealName)
    }

    pub fn identity(self) -> IdentityMode {
        match self {
            Self::CsvWithDiscord | Self::PdfWithDiscord => IdentityMode::WithDiscordName,
            Self::CsvRealName | Self::PdfRealName => IdentityMode::RealNameOnly,
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Self::CsvWithDiscord => "with-discord.csv",
            Self::CsvRealName => "real-name-only.csv",
            Self::PdfWithDiscord => "with-discord.pdf",
            Self::PdfRealName => "real-name-only.pdf",
        }
    }
}

// Create Message allows 25 MiB per request. Each request contains ONE attachment;
// reserve 1 MiB for multipart headers, the fixed filename and short message text.
// Apply the same conservative budget to preview followups as well.
pub(super) const FILE_BUDGET: usize = 24 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Generation,
    AttachmentLimit,
    RequestLimit,
    Delivery,
}

fn check_size(size: usize, attachment_limit: usize) -> Result<(), Failure> {
    if size > attachment_limit {
        Err(Failure::AttachmentLimit)
    } else if size > FILE_BUDGET {
        Err(Failure::RequestLimit)
    } else {
        Ok(())
    }
}

/// Generate and send CSVs before even starting PDF generation. Failures affect
/// only their own file. In particular, CSV-only never invokes the PDF renderer.
pub(super) async fn deliver<G, GF, S, SF>(
    format: ExportFormat,
    attachment_limit: usize,
    mut generate: G,
    mut send: S,
) -> Vec<(FileKind, Result<(), Failure>)>
where
    G: FnMut(FileKind) -> GF,
    GF: Future<Output = Result<Vec<u8>, Error>>,
    S: FnMut(FileKind, Vec<u8>) -> SF,
    SF: Future<Output = Result<(), Error>>,
{
    let mut results = Vec::new();
    for kind in [
        FileKind::CsvWithDiscord,
        FileKind::CsvRealName,
        FileKind::PdfWithDiscord,
        FileKind::PdfRealName,
    ] {
        if format == ExportFormat::Csv && kind.is_pdf() {
            continue;
        }
        let result = match generate(kind).await {
            Err(_) => Err(Failure::Generation),
            Ok(bytes) => match check_size(bytes.len(), attachment_limit) {
                Err(error) => Err(error),
                Ok(()) => send(kind, bytes).await.map_err(|_| Failure::Delivery),
            },
        };
        results.push((kind, result));
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn individual_and_request_limits_are_distinct_and_inclusive() {
        let individual = 20 * 1024 * 1024;
        assert_eq!(check_size(individual, individual), Ok(()));
        assert_eq!(
            check_size(individual + 1, individual),
            Err(Failure::AttachmentLimit)
        );
        assert_eq!(check_size(FILE_BUDGET, usize::MAX), Ok(()));
        assert_eq!(
            check_size(FILE_BUDGET + 1, usize::MAX),
            Err(Failure::RequestLimit)
        );
        assert_eq!(
            check_size(usize::MAX, usize::MAX),
            Err(Failure::RequestLimit)
        );
    }

    #[tokio::test]
    async fn csv_only_never_generates_pdf() {
        let results = deliver(
            ExportFormat::Csv,
            100,
            |kind| async move {
                assert!(!kind.is_pdf());
                Ok(vec![1])
            },
            |_, _| async { Ok(()) },
        )
        .await;
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|(_, result)| result.is_ok()));
    }

    #[tokio::test]
    async fn csvs_are_sent_before_pdf_failure_and_other_pdf_still_sends() {
        let events = Mutex::new(Vec::new());
        let results = deliver(
            ExportFormat::All,
            100,
            |kind| {
                events.lock().unwrap().push(("generate", kind));
                async move {
                    if kind == FileKind::PdfWithDiscord {
                        anyhow::bail!("dummy renderer failure");
                    }
                    Ok(vec![1])
                }
            },
            |kind, _| {
                events.lock().unwrap().push(("send", kind));
                async { Ok(()) }
            },
        )
        .await;
        assert_eq!(
            results,
            vec![
                (FileKind::CsvWithDiscord, Ok(())),
                (FileKind::CsvRealName, Ok(())),
                (FileKind::PdfWithDiscord, Err(Failure::Generation)),
                (FileKind::PdfRealName, Ok(())),
            ]
        );
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                ("generate", FileKind::CsvWithDiscord),
                ("send", FileKind::CsvWithDiscord),
                ("generate", FileKind::CsvRealName),
                ("send", FileKind::CsvRealName),
                ("generate", FileKind::PdfWithDiscord),
                ("generate", FileKind::PdfRealName),
                ("send", FileKind::PdfRealName),
            ]
        );
    }

    #[tokio::test]
    async fn oversized_file_and_send_failure_do_not_discard_other_files() {
        let sent = Mutex::new(Vec::new());
        let results = deliver(
            ExportFormat::All,
            10,
            |kind| async move {
                Ok(vec![
                    0;
                    if kind == FileKind::PdfWithDiscord {
                        11
                    } else {
                        1
                    }
                ])
            },
            |kind, _| {
                sent.lock().unwrap().push(kind);
                async move {
                    if kind == FileKind::CsvWithDiscord {
                        anyhow::bail!("dummy send failure");
                    }
                    Ok(())
                }
            },
        )
        .await;
        assert_eq!(
            results,
            vec![
                (FileKind::CsvWithDiscord, Err(Failure::Delivery)),
                (FileKind::CsvRealName, Ok(())),
                (FileKind::PdfWithDiscord, Err(Failure::AttachmentLimit)),
                (FileKind::PdfRealName, Ok(())),
            ]
        );
        assert_eq!(
            *sent.lock().unwrap(),
            vec![
                FileKind::CsvWithDiscord,
                FileKind::CsvRealName,
                FileKind::PdfRealName
            ]
        );
    }

    #[tokio::test]
    async fn two_large_pdfs_are_delivered_in_separate_requests() {
        let sizes = Mutex::new(Vec::new());
        let results = deliver(
            ExportFormat::All,
            20 * 1024 * 1024,
            |kind| async move {
                // CI's production-font PDF sizes: together they exceed 25 MiB.
                Ok(vec![
                    0;
                    match kind {
                        FileKind::PdfWithDiscord => 19_496_920,
                        FileKind::PdfRealName => 19_496_551,
                        _ => 10,
                    }
                ])
            },
            |_, bytes| {
                sizes.lock().unwrap().push(bytes.len());
                async { Ok(()) }
            },
        )
        .await;
        assert!(results.iter().all(|(_, result)| result.is_ok()));
        let sizes = sizes.lock().unwrap();
        assert_eq!(*sizes, vec![10, 10, 19_496_920, 19_496_551]);
        assert!(sizes.iter().sum::<usize>() > 25 * 1024 * 1024);
        assert!(sizes.iter().all(|size| *size <= FILE_BUDGET));
    }
}
