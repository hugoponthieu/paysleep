use chrono::{Datelike, Utc};
use paysleep_core::{Payslip, ReplicationState};
use serde::Serialize;
use tera::{Context, Tera, TeraResult};

pub struct Templates {
    tera: Tera,
}

impl Templates {
    pub fn new() -> TeraResult<Self> {
        let mut tera = Tera::new();
        tera.add_raw_template("index.html", include_str!("../templates/index.html"))?;
        Ok(Self { tera })
    }

    pub fn render_index(
        &self,
        payslips: &[Payslip],
        message: Option<(&str, &str)>,
        max_upload_mb: usize,
    ) -> TeraResult<String> {
        let current_year = Utc::now().year();
        let payslips = payslips.iter().map(PayslipView::from).collect::<Vec<_>>();
        let message = message.map(|(class, text)| MessageView { class, text });
        let months = [
            "Janvier",
            "Février",
            "Mars",
            "Avril",
            "Mai",
            "Juin",
            "Juillet",
            "Août",
            "Septembre",
            "Octobre",
            "Novembre",
            "Décembre",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, name)| MonthView {
            number: index + 1,
            name,
        })
        .collect::<Vec<_>>();
        let mut context = Context::new();
        context.insert("message", &message);
        context.insert("payslips", &payslips);
        context.insert("has_payslips", &!payslips.is_empty());
        context.insert("months", &months);
        context.insert("current_year", &current_year);
        context.insert("max_year", &(current_year + 1));
        context.insert("max_upload_mb", &max_upload_mb);
        self.tera.render("index.html", &context)
    }
}

#[derive(Serialize)]
struct MessageView<'a> {
    class: &'a str,
    text: &'a str,
}

#[derive(Serialize)]
struct MonthView<'a> {
    number: usize,
    name: &'a str,
}

#[derive(Serialize)]
struct PayslipView<'a> {
    id: uuid::Uuid,
    period: String,
    company: &'a str,
    original_name: &'a str,
    proton_class: &'static str,
    proton_text: &'static str,
    proton_error: Option<&'a str>,
    google_class: &'static str,
    google_text: &'static str,
    google_error: Option<&'a str>,
}

impl<'a> From<&'a Payslip> for PayslipView<'a> {
    fn from(payslip: &'a Payslip) -> Self {
        let (proton_class, proton_text, proton_error) = replication_view(&payslip.proton_state);
        let (google_class, google_text, google_error) = replication_view(&payslip.google_state);
        Self {
            id: payslip.id,
            period: format!("{:02}/{}", payslip.month, payslip.year),
            company: &payslip.company,
            original_name: &payslip.original_name,
            proton_class,
            proton_text,
            proton_error,
            google_class,
            google_text,
            google_error,
        }
    }
}

fn replication_view(state: &ReplicationState) -> (&'static str, &'static str, Option<&str>) {
    match state {
        ReplicationState::Disabled => ("", "Désactivée", None),
        ReplicationState::Synced => ("synced", "Synchronisée", None),
        ReplicationState::Failed(message) => ("failed", "Échec", Some(message.as_str())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_compiles_and_escapes_user_content() {
        let templates = Templates::new().unwrap();
        let payslip = Payslip {
            id: uuid::Uuid::nil(),
            company: "<script>".to_owned(),
            year: 2026,
            month: 4,
            original_name: "<b>.pdf".to_owned(),
            stored_name: "stored.pdf".to_owned(),
            size: 9,
            added_at: Utc::now(),
            proton_state: ReplicationState::Disabled,
            google_state: ReplicationState::Disabled,
        };
        let rendered = templates
            .render_index(&[payslip], Some(("error", "<strong>non</strong>")), 20)
            .unwrap();
        assert!(!rendered.contains("<script>"));
        assert!(!rendered.contains("<strong>non</strong>"));
        assert!(rendered.contains("&lt;script&gt;"));
    }
}
