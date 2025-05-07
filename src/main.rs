use chrono::{
    DateTime, Datelike, Duration, Month, NaiveDate, NaiveDateTime, ParseError, Timelike, Utc,
};
use clap::{Parser, Subcommand};
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Confirm};
use postgres::{Client, Error as PgError, NoTls};
use thiserror::Error; // Add colored for colored output

// WalletDB struct to manage database connection
struct WalletDB {
    client: Client,
}

#[derive(Error, Debug)]
pub enum WalletError {
    #[error("Database error: {0}")]
    Database(#[from] PgError),
    #[error("Invalid amount: {0}")]
    InvalidAmount(String),
    #[error("Ledger not found: {0}")]
    LedgerNotFound(String),
    #[error("Parse error: {0}")]
    ParseError(#[from] ParseError),
    #[error("Invalid date format: {0}")]
    InvalidDate(String),
    #[error("Date range error: {0}")]
    DateRangeError(String),
    #[error("Invalid month: {0}")]
    InvalidMonth(String),
    #[error("Invalid cap: {0}")]
    InvalidCap(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ReportPeriod {
    Today,
    Week,
    Month,
    All,
    Date(String),
    FromTo { from: String, to: String },
}
impl clap::ValueEnum for ReportPeriod {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Today, Self::Week, Self::Month, Self::All]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self {
            Self::Today => Some(clap::builder::PossibleValue::new("today")),
            Self::Week => Some(clap::builder::PossibleValue::new("week")),
            Self::Month => Some(clap::builder::PossibleValue::new("month")),
            Self::All => Some(clap::builder::PossibleValue::new("all")),
            Self::Date(_) => None,
            Self::FromTo { .. } => None,
        }
    }
}

impl WalletDB {
    fn new() -> Result<Self, WalletError> {
        // Connect to PostgreSQL
        let client = Client::connect(
            "host=localhost user=postgres password=postgres dbname=wallet_db",
            NoTls,
        )?;

        // Create tables if they don't exist

        Ok(WalletDB { client })
    }

    fn add_ledger(
        &mut self,
        code: &str,
        name: &str,
        description: &str,
        sort: &str,
        kind: &str,
    ) -> Result<(), WalletError> {
        self.client.execute(
            "INSERT INTO ledgers (code, name, description, sort, kind) VALUES ($1, $2, $3, $4, $5)",
            &[&code, &name, &description, &sort, &kind],
        )?;
        println!("Added ledger: {} - {}", code, name);
        Ok(())
    }

    fn retrieve_ledger_id(&mut self, code: &str) -> Result<i32, WalletError> {
        let row = self
            .client
            .query_one("SELECT id FROM ledgers WHERE code = $1", &[&code])?;
        Ok(row.get(0))
    }

    fn proceed_spend(
        &mut self,
        patron: &str,
        outlay: &str,
        amount: f64,
        narration: &str,
        created_at: Option<NaiveDateTime>,
    ) -> Result<(), WalletError> {
        if amount <= 0.0 {
            return Err(WalletError::InvalidAmount(
                "Amount must be positive".to_string(),
            ));
        }

        let patron_id = self.retrieve_ledger_id(patron)?;
        let outlay_id = self.retrieve_ledger_id(outlay)?;

        if let Some(created_at) = created_at {
            // Use the provided created_at date for both created_at and updated_at
            self.client.execute(
                "INSERT INTO proceedings (cr_from, db_to, amount, narration, created_at) 
                 VALUES ($1, $2, $3, $4, $5)",
                &[&patron_id, &outlay_id, &amount, &narration, &created_at],
            )?;
        } else {
            // Let the database set created_at and updated_at to CURRENT_TIMESTAMP
            self.client.execute(
                "INSERT INTO proceedings (cr_from, db_to, amount, narration) VALUES ($1, $2, $3, $4)",
                &[&patron_id, &outlay_id, &amount, &narration],
            )?;
        }

        println!(
            "Added spending: {} -> {}: {} ({})",
            patron, outlay, amount, narration
        );
        Ok(())
    }

    fn generate_spending_report(&mut self, period: ReportPeriod) -> Result<(), WalletError> {
        let now: DateTime<Utc> = Utc::now();
        let (start_date_naive, end_date_naive, period_str): (
            NaiveDateTime,
            Option<NaiveDateTime>,
            String,
        ) = match &period {
            ReportPeriod::Today => {
                let start = now
                    .with_hour(0)
                    .and_then(|d| d.with_minute(0))
                    .and_then(|d| d.with_second(0))
                    .and_then(|d| d.with_nanosecond(0))
                    .unwrap();
                (start.naive_utc(), None, "Today".to_string())
            }
            ReportPeriod::Week => {
                let start = now - Duration::days(now.weekday().num_days_from_monday() as i64)
                    + Duration::hours(0)
                    - Duration::minutes(now.minute() as i64)
                    - Duration::seconds(now.second() as i64)
                    - Duration::nanoseconds(now.nanosecond() as i64);
                (start.naive_utc(), None, "This Week".to_string())
            }
            ReportPeriod::Month => {
                let start = now
                    .with_day(1)
                    .and_then(|d| d.with_hour(0))
                    .and_then(|d| d.with_minute(0))
                    .and_then(|d| d.with_second(0))
                    .and_then(|d| d.with_nanosecond(0))
                    .unwrap();
                (start.naive_utc(), None, "This Month".to_string())
            }
            ReportPeriod::All => {
                let start =
                    NaiveDateTime::parse_from_str("1970-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")?;
                (start, None, "All Time".to_string())
            }
            ReportPeriod::Date(date_str) => {
                let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").map_err(|_| {
                    WalletError::InvalidDate(format!(
                        "Invalid date format: {}. Use YYYY-MM-DD",
                        date_str
                    ))
                })?;
                let start = date.and_hms_opt(0, 0, 0).unwrap();
                let end = date.and_hms_opt(23, 59, 59).unwrap();
                (start, Some(end), format!("Date: {}", date_str))
            }
            ReportPeriod::FromTo { from, to } => {
                let from_date = NaiveDate::parse_from_str(from, "%Y-%m-%d").map_err(|_| {
                    WalletError::InvalidDate(format!(
                        "Invalid 'from' date format: {}. Use YYYY-MM-DD",
                        from
                    ))
                })?;
                let to_date = NaiveDate::parse_from_str(to, "%Y-%m-%d").map_err(|_| {
                    WalletError::InvalidDate(format!(
                        "Invalid 'to' date format: {}. Use YYYY-MM-DD",
                        to
                    ))
                })?;
                if from_date > to_date {
                    return Err(WalletError::DateRangeError(
                        "The 'from' date must be earlier than or equal to the 'to' date."
                            .to_string(),
                    ));
                }
                let start = from_date.and_hms_opt(0, 0, 0).unwrap();
                let end = to_date.and_hms_opt(23, 59, 59).unwrap();
                (start, Some(end), format!("From {} to {}", from, to))
            }
        };

        let query = match &period {
            ReportPeriod::All => {
                "
                SELECT 
                    l.code, 
                    l.name, 
                    CASE 
                        WHEN l.kind = 'LIABILITY' THEN 
                            COALESCE((
                                SELECT SUM(p1.amount) 
                                FROM proceedings p1 
                                WHERE p1.db_to = l.id
                            ), 0) - COALESCE((
                                SELECT SUM(p2.amount) 
                                FROM proceedings p2 
                                WHERE p2.cr_from = l.id
                            ), 0)
                        ELSE 
                            COALESCE((
                                SELECT SUM(p3.amount) 
                                FROM proceedings p3 
                                WHERE p3.db_to = l.id
                            ), 0)
                    END as amount
                FROM ledgers l
                ORDER BY amount DESC
            "
            }
            ReportPeriod::Date(_) => {
                "
                SELECT 
                    l.code, 
                    l.name, 
                    CASE 
                        WHEN l.kind = 'LIABILITY' THEN 
                            COALESCE((
                                SELECT SUM(p1.amount) 
                                FROM proceedings p1 
                                WHERE p1.db_to = l.id 
                                AND p1.created_at >= $1 AND p1.created_at <= $2
                            ), 0) - COALESCE((
                                SELECT SUM(p2.amount) 
                                FROM proceedings p2 
                                WHERE p2.cr_from = l.id 
                                AND p2.created_at >= $1 AND p2.created_at <= $2
                            ), 0)
                        ELSE 
                            COALESCE((
                                SELECT SUM(p3.amount) 
                                FROM proceedings p3 
                                WHERE p3.db_to = l.id 
                                AND p3.created_at >= $1 AND p3.created_at <= $2
                            ), 0)
                    END as amount
                FROM ledgers l
                ORDER BY amount DESC
            "
            }
            _ => {
                "
                SELECT 
                    l.code, 
                    l.name, 
                    CASE 
                        WHEN l.kind = 'LIABILITY' THEN 
                            COALESCE((
                                SELECT SUM(p1.amount) 
                                FROM proceedings p1 
                                WHERE p1.db_to = l.id 
                                AND p1.created_at >= $1
                            ), 0) - COALESCE((
                                SELECT SUM(p2.amount) 
                                FROM proceedings p2 
                                WHERE p2.cr_from = l.id 
                                AND p2.created_at >= $1
                            ), 0)
                        ELSE 
                            COALESCE((
                                SELECT SUM(p3.amount) 
                                FROM proceedings p3 
                                WHERE p3.db_to = l.id 
                                AND p3.created_at >= $1
                            ), 0)
                    END as amount
                FROM ledgers l
                ORDER BY amount DESC
            "
            }
        };
        let rows = match &period {
            ReportPeriod::All => self.client.query(query, &[])?,
            ReportPeriod::Date(_) => self
                .client
                .query(query, &[&start_date_naive, &end_date_naive.unwrap()])?,
            _ => self.client.query(query, &[&start_date_naive])?,
        };

        println!("\nSpending Report ({}):", period_str);
        println!("{:<10} {:<30} {:<15}", "Code", "Name", "Net Amount");
        println!("{:-<55}", "");
        let mut grand_total: f64 = 0.0;
        for row in rows.iter() {
            let code: String = row.get(0);
            let name: String = row.get(1);
            let net_amount: f64 = row.get(2);
            grand_total += net_amount;
            println!("{:<10} {:<30} {:<15.2}", code, name, net_amount);
        }
        println!("{:-<55}", "");
        println!("{:<40} {:<15.2}", "Grand Total", grand_total);
        Ok(())
    }
    fn generate_ledger_report(
        &mut self,
        ledger_code: &str,
        period: ReportPeriod,
    ) -> Result<(), WalletError> {
        let ledger_id = self.retrieve_ledger_id(ledger_code)?;
        let ledger_name: String = self
            .client
            .query_one("SELECT name FROM ledgers WHERE id = $1", &[&ledger_id])?
            .get(0);

        let now: DateTime<Utc> = Utc::now();
        let (start_date_naive, end_date_naive, period_str): (
            NaiveDateTime,
            Option<NaiveDateTime>,
            String,
        ) = match &period {
            ReportPeriod::Today => {
                let start = now
                    .with_hour(0)
                    .and_then(|d| d.with_minute(0))
                    .and_then(|d| d.with_second(0))
                    .and_then(|d| d.with_nanosecond(0))
                    .unwrap();
                (start.naive_utc(), None, "Today".to_string())
            }
            ReportPeriod::Week => {
                let start = now - Duration::days(now.weekday().num_days_from_monday() as i64)
                    + Duration::hours(0)
                    - Duration::minutes(now.minute() as i64)
                    - Duration::seconds(now.second() as i64)
                    - Duration::nanoseconds(now.nanosecond() as i64);
                (start.naive_utc(), None, "This Week".to_string())
            }
            ReportPeriod::Month => {
                let start = now
                    .with_day(1)
                    .and_then(|d| d.with_hour(0))
                    .and_then(|d| d.with_minute(0))
                    .and_then(|d| d.with_second(0))
                    .and_then(|d| d.with_nanosecond(0))
                    .unwrap();
                (start.naive_utc(), None, "This Month".to_string())
            }
            ReportPeriod::All => {
                let start =
                    NaiveDateTime::parse_from_str("1970-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")?;
                (start, None, "All Time".to_string())
            }
            ReportPeriod::Date(date_str) => {
                let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").map_err(|_| {
                    WalletError::InvalidDate(format!(
                        "Invalid date format: {}. Use YYYY-MM-DD",
                        date_str
                    ))
                })?;
                let start = date.and_hms_opt(0, 0, 0).unwrap();
                let end = date.and_hms_opt(23, 59, 59).unwrap();
                (start, Some(end), format!("Date: {}", date_str))
            }
            ReportPeriod::FromTo { from, to } => {
                let from_date = NaiveDate::parse_from_str(from, "%Y-%m-%d").map_err(|_| {
                    WalletError::InvalidDate(format!(
                        "Invalid 'from' date format: {}. Use YYYY-MM-DD",
                        from
                    ))
                })?;
                let to_date = NaiveDate::parse_from_str(to, "%Y-%m-%d").map_err(|_| {
                    WalletError::InvalidDate(format!(
                        "Invalid 'to' date format: {}. Use YYYY-MM-DD",
                        to
                    ))
                })?;
                if from_date > to_date {
                    return Err(WalletError::DateRangeError(
                        "The 'from' date must be earlier than or equal to the 'to' date."
                            .to_string(),
                    ));
                }
                let start = from_date.and_hms_opt(0, 0, 0).unwrap();
                let end = to_date.and_hms_opt(23, 59, 59).unwrap();
                (start, Some(end), format!("From {} to {}", from, to))
            }
        };

        let query = match &period {
            ReportPeriod::All => {
                "
                SELECT p.created_at, 
                       CASE 
                           WHEN p.cr_from = $1 THEN (SELECT code FROM ledgers WHERE id = p.db_to)
                           ELSE (SELECT code FROM ledgers WHERE id = p.cr_from)
                       END as counterparty,
                       p.narration,
                       CASE WHEN p.cr_from = $1 THEN p.amount ELSE 0 END as credit_amount,
                       CASE WHEN p.db_to = $1 THEN p.amount ELSE 0 END as debit_amount
                FROM proceedings p
                WHERE p.cr_from = $1 OR p.db_to = $1
                ORDER BY p.created_at DESC
            "
            }
            ReportPeriod::Date(_) | ReportPeriod::FromTo { .. } => {
                "
                SELECT p.created_at, 
                       CASE 
                           WHEN p.cr_from = $1 THEN (SELECT code FROM ledgers WHERE id = p.db_to)
                           ELSE (SELECT code FROM ledgers WHERE id = p.cr_from)
                       END as counterparty,
                       p.narration,
                       CASE WHEN p.cr_from = $1 THEN p.amount ELSE 0 END as credit_amount,
                       CASE WHEN p.db_to = $1 THEN p.amount ELSE 0 END as debit_amount
                FROM proceedings p
                WHERE (p.cr_from = $1 OR p.db_to = $1) AND p.created_at >= $2 AND p.created_at <= $3
                ORDER BY p.created_at DESC
            "
            }
            _ => {
                "
                SELECT p.created_at, 
                       CASE 
                           WHEN p.cr_from = $1 THEN (SELECT code FROM ledgers WHERE id = p.db_to)
                           ELSE (SELECT code FROM ledgers WHERE id = p.cr_from)
                       END as counterparty,
                       p.narration,
                       CASE WHEN p.cr_from = $1 THEN p.amount ELSE 0 END as credit_amount,
                       CASE WHEN p.db_to = $1 THEN p.amount ELSE 0 END as debit_amount
                FROM proceedings p
                WHERE (p.cr_from = $1 OR p.db_to = $1) AND p.created_at >= $2
                ORDER BY p.created_at DESC
            "
            }
        };

        let rows = match &period {
            ReportPeriod::All => self.client.query(query, &[&ledger_id])?,
            ReportPeriod::Date(_) | ReportPeriod::FromTo { .. } => self.client.query(
                query,
                &[&ledger_id, &start_date_naive, &end_date_naive.unwrap()],
            )?,
            _ => self.client.query(query, &[&ledger_id, &start_date_naive])?,
        };

        println!(
            "\nLedger Report for {} - {} ({}):",
            ledger_code, ledger_name, period_str
        );
        println!(
            "{:<20} {:<10} {:<30} {:<15} {:<15}",
            "Date", "Counterparty", "Narration", "Credit", "Debit"
        );
        println!("{:-<90}", "");

        let mut total_credits: f64 = 0.0;
        let mut total_debits: f64 = 0.0;

        for row in rows.iter() {
            let created_at: NaiveDateTime = row.get(0);
            let counterparty: String = row.get(1);
            let narration: String = row.get(2);
            let credit_amount: f64 = row.get(3);
            let debit_amount: f64 = row.get(4);

            total_credits += credit_amount;
            total_debits += debit_amount;

            println!(
                "{:<20} {:<10} {:<30} {:<15.2} {:<15.2}",
                created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                counterparty,
                narration,
                credit_amount,
                debit_amount
            );
        }

        let net_balance = total_debits - total_credits;

        println!("{:-<90}", "");
        println!(
            "{:<60} {:<15.2} {:<15.2}",
            "Totals", total_credits, total_debits
        );
        println!(
            "{:<60} {:<15.2}",
            "Net Balance (Debits - Credits)", net_balance
        );

        Ok(())
    }
    fn generate_recent_transactions_report(&mut self) -> Result<(), WalletError> {
        let query = "
            SELECT p.created_at, 
                   (SELECT code FROM ledgers WHERE id = p.cr_from) as cr_from_code,
                   (SELECT code FROM ledgers WHERE id = p.db_to) as db_to_code,
                   p.amount,
                   p.narration
            FROM proceedings p
            ORDER BY p.created_at DESC
            LIMIT 10
        ";

        let rows = self.client.query(query, &[])?;

        println!("\nRecent Transactions Report (Last 10):");
        println!(
            "{:<20} {:<10} {:<10} {:<15} {:<30}",
            "Date", "From", "To", "Amount", "Narration"
        );
        println!("{:-<85}", "");

        for row in rows.iter() {
            let created_at: NaiveDateTime = row.get(0);
            let cr_from_code: String = row.get(1);
            let db_to_code: String = row.get(2);
            let amount: f64 = row.get(3);
            let narration: String = row.get(4);

            println!(
                "{:<20} {:<10} {:<10} {:<15.2} {:<30}",
                created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                cr_from_code,
                db_to_code,
                amount,
                narration
            );
        }

        println!("{:-<85}", "");
        Ok(())
    }

    // fn generate_calendar_report(&mut self) -> Result<(), WalletError> {
    //     let now: DateTime<Utc> = Utc::now();
    //     // Start of the month
    //     let start_date = now
    //         .with_day(1)
    //         .and_then(|d| d.with_hour(0))
    //         .and_then(|d| d.with_minute(0))
    //         .and_then(|d| d.with_second(0))
    //         .and_then(|d| d.with_nanosecond(0))
    //         .unwrap()
    //         .naive_utc();
    //     // End of today
    //     let end_date = now
    //         .with_hour(23)
    //         .and_then(|d| d.with_minute(59))
    //         .and_then(|d| d.with_second(59))
    //         .and_then(|d| d.with_nanosecond(999_999_999))
    //         .unwrap()
    //         .naive_utc();

    //     // Query to get daily totals
    // let query = "
    //     SELECT
    //         DATE(p.created_at) as day,
    //         SUM(CASE
    //                 WHEN l.kind = 'LIABILITY' THEN
    //                     (CASE WHEN p.db_to = l.id THEN p.amount ELSE 0 END) -
    //                     (CASE WHEN p.cr_from = l.id THEN p.amount ELSE 0 END)
    //                 ELSE
    //                     CASE WHEN p.db_to = l.id THEN p.amount ELSE 0 END
    //             END) as daily_amount
    //     FROM proceedings p
    //     JOIN ledgers l ON p.db_to = l.id OR p.cr_from = l.id
    //     WHERE p.created_at >= $1 AND p.created_at <= $2
    //     GROUP BY DATE(p.created_at)
    //     HAVING SUM(CASE
    //                    WHEN l.kind = 'LIABILITY' THEN
    //                        (CASE WHEN p.db_to = l.id THEN p.amount ELSE 0 END) -
    //                        (CASE WHEN p.cr_from = l.id THEN p.amount ELSE 0 END)
    //                    ELSE
    //                        CASE WHEN p.db_to = l.id THEN p.amount ELSE 0 END
    //                END) != 0
    //     ORDER BY DATE(p.created_at)
    // ";

    //     let rows = self.client.query(query, &[&start_date, &end_date])?;

    //     // Get the month name for the report header
    //     let month_name = now.format("%B %Y").to_string();
    //     println!("\nDaily Spending Report for {}:", month_name);
    //     println!("{:<15} {:<15}", "Date", "Total Spent");
    //     println!("{:-<30}", "");

    //     let mut grand_total: f64 = 0.0;
    //     for row in rows.iter() {
    //         let day: NaiveDate = row.get(0);
    //         let daily_amount: f64 = row.get(1);
    //         grand_total += daily_amount;
    //         println!(
    //             "{:<15} {:<15.2}",
    //             day.format("%Y-%m-%d").to_string(),
    //             daily_amount
    //         );
    //     }

    //     println!("{:-<30}", "");
    //     println!("{:<15} {:<15.2}", "Grand Total", grand_total);
    //     Ok(())
    // }

    fn generate_calendar_report(
        &mut self,
        month_arg: Option<&str>,
        cap: Option<f64>,
    ) -> Result<(), WalletError> {
        let now: DateTime<Utc> = Utc::now();
        let current_year = now.year();
        let current_month = now.month();

        // Parse the month if provided, otherwise use the current month
        let (target_month, target_year, month_name) = match month_arg {
            Some(month_str) => {
                // Parse the month name (case-insensitive)
                let month_str_lower = month_str.to_lowercase();
                let month = match month_str_lower.as_str() {
                    "january" => Month::January,
                    "february" => Month::February,
                    "march" => Month::March,
                    "april" => Month::April,
                    "may" => Month::May,
                    "june" => Month::June,
                    "july" => Month::July,
                    "august" => Month::August,
                    "september" => Month::September,
                    "october" => Month::October,
                    "november" => Month::November,
                    "december" => Month::December,
                    _ => {
                        return Err(WalletError::InvalidMonth(format!(
                            "Invalid month: {}. Use full month name (e.g., 'April').",
                            month_str
                        )))
                    }
                };
                let month_number = month.number_from_month();
                // Determine the year: if the target month is in the future, use the previous year
                let year = if month_number > current_month {
                    current_year - 1
                } else {
                    current_year
                };
                (month_number, year, month.name().to_string())
            }
            None => (current_month, current_year, now.format("%B").to_string()),
        };

        // Start of the month
        let start_date = NaiveDate::from_ymd_opt(target_year, target_month, 1)
            .ok_or_else(|| WalletError::InvalidDate("Failed to construct start date".to_string()))?
            .and_hms_opt(0, 0, 0)
            .unwrap();

        // End of the month: if it's the current month, end at the current day; otherwise, use the last day of the month
        let end_date = if target_month == current_month && target_year == current_year {
            // End at the end of today
            now.with_hour(23)
                .and_then(|d| d.with_minute(59))
                .and_then(|d| d.with_second(59))
                .and_then(|d| d.with_nanosecond(999_999_999))
                .unwrap()
                .naive_utc()
        } else {
            // Find the last day of the target month
            let next_month = if target_month == 12 {
                NaiveDate::from_ymd_opt(target_year + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(target_year, target_month + 1, 1)
            }
            .ok_or_else(|| {
                WalletError::InvalidDate("Failed to construct next month date".to_string())
            })?;
            next_month
                .pred_opt()
                .unwrap()
                .and_hms_opt(23, 59, 59)
                .unwrap()
        };

        let query = "
        SELECT
            DATE(p.created_at) as day,
            SUM(CASE
                    WHEN l.kind = 'LIABILITY' THEN
                        (CASE WHEN p.db_to = l.id THEN p.amount ELSE 0 END) -
                        (CASE WHEN p.cr_from = l.id THEN p.amount ELSE 0 END)
                    ELSE
                        CASE WHEN p.db_to = l.id THEN p.amount ELSE 0 END
                END) as daily_amount
        FROM proceedings p
        JOIN ledgers l ON p.db_to = l.id OR p.cr_from = l.id
        WHERE p.created_at >= $1 AND p.created_at <= $2
        GROUP BY DATE(p.created_at)
        HAVING SUM(CASE
                       WHEN l.kind = 'LIABILITY' THEN
                           (CASE WHEN p.db_to = l.id THEN p.amount ELSE 0 END) -
                           (CASE WHEN p.cr_from = l.id THEN p.amount ELSE 0 END)
                       ELSE
                           CASE WHEN p.db_to = l.id THEN p.amount ELSE 0 END
                   END) != 0
        ORDER BY DATE(p.created_at)
    ";

        // Query to get daily totals, focusing on debits to non-liability ledgers
        // let query = "
        //     SELECT
        //         DATE(p.created_at) as day,
        //         SUM(p.amount) as daily_amount
        //     FROM proceedings p
        //     JOIN ledgers l ON p.db_to = l.id
        //     WHERE p.created_at >= $1 AND p.created_at <= $2
        //         AND l.kind != 'LIABILITY'
        //     GROUP BY DATE(p.created_at)
        //     HAVING SUM(p.amount) > 0
        //     ORDER BY DATE(p.created_at)
        // ";

        let rows = self.client.query(query, &[&start_date, &end_date])?;

        // Format the report header with the month and year
        let mut report_header = format!("{} {}", month_name, target_year);
        if let Some(cap_value) = cap {
            report_header = format!("{} (Daily Cap: {:.2})", report_header, cap_value);
        }
        println!("\nDaily Spending Report for {}:", report_header);
        // Update the header to include a "Difference" column if a cap is specified
        if cap.is_some() {
            println!("{:<15} {:<15} {:<15}", "Date", "Total Spent", "Skimp");
            println!("{:-<45}", "");
        } else {
            println!("{:<15} {:<15}", "Date", "Total Spent");
            println!("{:-<30}", "");
        }

        let mut grand_total: f64 = 0.0;
        let mut skimp: f64 = 0.0;
        for row in rows.iter() {
            let day: NaiveDate = row.get(0);
            let daily_amount: f64 = row.get(1);
            grand_total += daily_amount;

            if let Some(cap_value) = cap {
                let difference = cap_value - daily_amount;
                let difference_str = if difference > 0.0 {
                    skimp += difference;
                    // Underspent: show in green
                    format!("{:.2}", difference).green()
                } else {
                    // Overspent: show in red
                    format!("{:.2}", difference).red()
                };
                println!(
                    "{:<15} {:<15.2} {:<15}",
                    day.format("%Y-%m-%d").to_string(),
                    daily_amount,
                    difference_str
                );
            } else {
                println!(
                    "{:<15} {:<15.2}",
                    day.format("%Y-%m-%d").to_string(),
                    daily_amount
                );
            }
        }

        if cap.is_some() {
            println!("{:-<45}", "");
        } else {
            println!("{:-<30}", "");
        }
        println!("{:<15} {:<15.2} {:<15}", "Grand Total", grand_total, skimp);

        Ok(())
    }

    // New method to list all ledgers (helpful for debugging or user reference)
    fn list_ledgers(&mut self) -> Result<(), WalletError> {
        let rows = self.client.query(
            "SELECT code, name, sort, kind FROM ledgers ORDER BY code",
            &[],
        )?;

        println!("\nList of Ledgers:");
        println!(
            "{:<10} {:<30} {:<10} {:<10}",
            "Code", "Name", "Sort", "Kind"
        );
        println!("{:-<60}", "");
        for row in rows {
            let code: String = row.get(0);
            let name: String = row.get(1);
            let sort: String = row.get(2);
            let kind: String = row.get(3);
            println!("{:<10} {:<30} {:<10} {:<10}", code, name, sort, kind);
        }
        Ok(())
    }

    fn setup_db(&mut self) -> Result<(), WalletError> {
        self.client.batch_execute(
            "
            CREATE TABLE IF NOT EXISTS ledgers (
                id SERIAL PRIMARY KEY,
                code VARCHAR(10) NOT NULL,
                name VARCHAR(100) NOT NULL,
                description TEXT,
                sort VARCHAR(10) NOT NULL,
                kind VARCHAR(20) NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS proceedings (
                id SERIAL PRIMARY KEY,
                cr_from INTEGER NOT NULL REFERENCES ledgers(id),
                db_to INTEGER NOT NULL REFERENCES ledgers(id),
                amount DOUBLE PRECISION NOT NULL,
                narration TEXT NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
            ",
        )?;
        print!("Db setup completed successfully");
        Ok(())
    }
    fn clear_tables(&mut self) -> Result<(), WalletError> {
        self.client.execute("DELETE FROM proceedings", &[])?;
        self.client.execute("DELETE FROM ledgers", &[])?;
        println!("All data cleared from ledgers and proceedings tables.");
        Ok(())
    }
}

// CLI commands
#[derive(Parser)]
#[command(name = "wallet")]
#[command(about = "A simple wallet management CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new ledger
    AddLedger {
        code: String,
        name: String,
        description: String,
        sort: String,
        kind: String,
    },
    /// Add a new spending entry
    Spend {
        patron: String,
        outlay: String,
        amount: f64,
        narration: String,
        #[arg(long)]
        date: Option<String>,
    },
    /// Generate a spending report
    Report {
        #[arg(value_enum)]
        period: Option<ReportPeriod>,
        #[arg(long)]
        date: Option<String>,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
    },
    // SummaryReport {
    //     #[arg(value_enum, default_value_t = ReportPeriod::All)]
    //     period: ReportPeriod,
    // },
    LedgerReport {
        code: String,
        #[arg(value_enum)]
        period: Option<ReportPeriod>,
        #[arg(long)]
        date: Option<String>,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
    },
    /// List all ledgers
    Calendar {
        #[arg(
            help = "Month name (e.g., 'april') or cap value (e.g., '500') if used without month"
        )]
        month: Option<String>,
        #[arg(help = "Daily spending cap (e.g., '500')")]
        cap: Option<String>,
    },
    ListLedgers,
    Last,
    DbSetup,
    Clear,
}

fn main() -> Result<(), WalletError> {
    let cli = Cli::parse();

    // Initialize the database
    let mut db = WalletDB::new()?;

    match cli.command {
        Commands::AddLedger {
            code,
            name,
            description,
            sort,
            kind,
        } => {
            db.add_ledger(&code, &name, &description, &sort, &kind)
                .map_err(|e| {
                    eprintln!("Failed to add ledger: {}", e);
                    e
                })?;
        }
        Commands::Spend {
            patron,
            outlay,
            amount,
            narration,
            date,
        } => {
            let created_at = if let Some(date_str) = date {
                // Parse the date string (e.g., "2025-04-20") into a NaiveDate
                let naive_date =
                    NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").map_err(|_| {
                        WalletError::InvalidDate(format!(
                            "Invalid date format: {}. Use YYYY-MM-DD",
                            date_str
                        ))
                    })?;
                // Convert to NaiveDateTime by setting time to 00:00:00
                Some(naive_date.and_hms_opt(0, 0, 0).unwrap())
            } else {
                None
            };
            db.proceed_spend(&patron, &outlay, amount, &narration, created_at)
                .map_err(|e| {
                    eprintln!("Failed to record spending: {}", e);
                    e
                })?;
        }
        Commands::Report {
            period,
            date,
            from,
            to,
        } => {
            let period = match (period, date, from, to) {
                (Some(p), None, None, None) => p,
                (None, Some(date), None, None) => ReportPeriod::Date(date),
                (None, None, Some(from), Some(to)) => ReportPeriod::FromTo { from, to },
                (None, None, None, None) => ReportPeriod::All, // Default to All if nothing is specified
                (Some(_), Some(_), _, _) => {
                    return Err(WalletError::InvalidDate(
                        "Cannot specify both a period and a date. Use either 'spendlog report <period>' or 'spendlog report --date <YYYY-MM-DD>'.".to_string(),
                    ));
                }
                (Some(_), _, Some(_), Some(_)) => {
                    return Err(WalletError::InvalidDate(
                        "Cannot specify both a period and a date range. Use either 'spendlog report <period>' or 'spendlog report --from <YYYY-MM-DD> --to <YYYY-MM-DD>'.".to_string(),
                    ));
                }
                (None, None, Some(_), None) | (None, None, None, Some(_)) => {
                    return Err(WalletError::InvalidDate(
                        "Must specify both --from and --to dates for a date range.".to_string(),
                    ));
                }
                _ => {
                    return Err(WalletError::InvalidDate(
                        "Invalid combination of arguments. Use 'spendlog report <period>', 'spendlog report --date <YYYY-MM-DD>', or 'spendlog report --from <YYYY-MM-DD> --to <YYYY-MM-DD>'.".to_string(),
                    ));
                }
            };
            db.generate_spending_report(period).map_err(|e| {
                eprintln!("Failed to generate report: {}", e);
                e
            })?;
        }
        Commands::LedgerReport {
            code,
            period,
            date,
            from,
            to,
        } => {
            let period = match (period, date, from, to) {
                (Some(p), None, None, None) => p,
                (None, Some(date), None, None) => ReportPeriod::Date(date),
                (None, None, Some(from), Some(to)) => ReportPeriod::FromTo { from, to },
                (None, None, None, None) => ReportPeriod::All, // Default to All if nothing is specified
                (Some(_), Some(_), _, _) => {
                    return Err(WalletError::InvalidDate(
                        "Cannot specify both a period and a date. Use either 'spendlog ledger-report <code> <period>' or 'spendlog ledger-report <code> --date <YYYY-MM-DD>'.".to_string(),
                    ));
                }
                (Some(_), _, Some(_), Some(_)) => {
                    return Err(WalletError::InvalidDate(
                        "Cannot specify both a period and a date range. Use either 'spendlog ledger-report <code> <period>' or 'spendlog ledger-report <code> --from <YYYY-MM-DD> --to <YYYY-MM-DD>'.".to_string(),
                    ));
                }
                (None, None, Some(_), None) | (None, None, None, Some(_)) => {
                    return Err(WalletError::InvalidDate(
                        "Must specify both --from and --to dates for a date range.".to_string(),
                    ));
                }
                _ => {
                    return Err(WalletError::InvalidDate(
                        "Invalid combination of arguments. Use 'spendlog ledger-report <code> <period>', 'spendlog ledger-report <code> --date <YYYY-MM-DD>', or 'spendlog ledger-report <code> --from <YYYY-MM-DD> --to <YYYY-MM-DD>'.".to_string(),
                    ));
                }
            };
            db.generate_ledger_report(&code, period).map_err(|e| {
                eprintln!("Failed to generate ledger report: {}", e);
                e
            })?;
        }

        Commands::ListLedgers => {
            db.list_ledgers().map_err(|e| {
                eprintln!("Failed to list ledgers: {}", e);
                e
            })?;
        }
        Commands::Calendar { month, cap } => {
            // Determine if the month argument is actually a cap value
            let (month_arg, cap_value) = match (month.clone(), cap) {
                (Some(m), Some(c)) => {
                    // Both month and cap are provided
                    let cap_num = c.parse::<f64>().map_err(|_| {
                        WalletError::InvalidCap(format!(
                            "Invalid cap value: {}. Must be a number.",
                            c
                        ))
                    })?;
                    if cap_num <= 0.0 {
                        return Err(WalletError::InvalidCap(
                            "Cap must be a positive number.".to_string(),
                        ));
                    }
                    (Some(m), Some(cap_num))
                }
                (Some(m), None) => {
                    // Check if 'm' is a number (cap) or a month
                    if let Ok(cap_num) = m.parse::<f64>() {
                        if cap_num <= 0.0 {
                            return Err(WalletError::InvalidCap(
                                "Cap must be a positive number.".to_string(),
                            ));
                        }
                        (None, Some(cap_num))
                    } else {
                        (Some(m), None)
                    }
                }
                (None, Some(c)) => {
                    let cap_num = c.parse::<f64>().map_err(|_| {
                        WalletError::InvalidCap(format!(
                            "Invalid cap value: {}. Must be a number.",
                            c
                        ))
                    })?;
                    if cap_num <= 0.0 {
                        return Err(WalletError::InvalidCap(
                            "Cap must be a positive number.".to_string(),
                        ));
                    }
                    (None, Some(cap_num))
                }
                (None, None) => (None, None),
            };

            db.generate_calendar_report(month_arg.as_deref(), cap_value)
                .map_err(|e| {
                    eprintln!("Failed to generate calendar report: {}", e);
                    e
                })?;
        }
        Commands::Last => {
            db.generate_recent_transactions_report().map_err(|e| {
                eprintln!("Failed to generate recent transactions report: {}", e);
                e
            })?;
        }
        Commands::DbSetup => {
            db.setup_db()?;
        }
        Commands::Clear => {
            let confirmed = Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt("Are you sure you want to delete all data from the database? This action cannot be undone.")
                .default(false)
                .interact()
                .unwrap_or(false);

            if confirmed {
                db.clear_tables().map_err(|e| {
                    eprintln!("Failed to clear tables: {}", e);
                    e
                })?;
            } else {
                println!("Operation canceled. No data was deleted.");
            }
        }
    }

    Ok(())
}
