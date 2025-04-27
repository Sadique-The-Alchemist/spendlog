use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, ParseError, Timelike, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use dialoguer::{theme::ColorfulTheme, Confirm};
use postgres::{Client, Error as PgError, NoTls};
use thiserror::Error;

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
}

#[derive(ValueEnum, Clone, Debug, PartialEq)]
enum ReportPeriod {
    #[clap(name = "today")]
    Today,
    #[clap(name = "week")]
    Week,
    #[clap(name = "month")]
    Month,
    #[clap(name = "all")]
    All,
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

    // fn proceed_spend(
    //     &mut self,
    //     patron: &str,
    //     outlay: &str,
    //     amount: f64,
    //     narration: &str,
    // ) -> Result<(), WalletError> {
    //     // Validate amount
    //     if amount <= 0.0 {
    //         return Err(WalletError::InvalidAmount(
    //             "Amount must be positive".to_string(),
    //         ));
    //     }

    //     // Retrieve ledger IDs
    //     let patron_id = self.retrieve_ledger_id(patron)?;
    //     let outlay_id = self.retrieve_ledger_id(outlay)?;

    //     // Insert the spending entry
    //     self.client.execute(
    //         "INSERT INTO proceedings (cr_from, db_to, amount, narration) VALUES ($1, $2, $3, $4)",
    //         &[&patron_id, &outlay_id, &amount, &narration],
    //     )?;
    //     println!(
    //         "Added spending: {} -> {}: {} ({})",
    //         patron, outlay, amount, narration
    //     );
    //     Ok(())
    // }

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

    // fn generate_spending_report(&mut self, period: ReportPeriod) -> Result<(), PgError> {
    //     // Report: Total spending by outlay (db_to ledger)
    //     let rows = self.client.query(
    //         "
    //         SELECT l.code, l.name, SUM(p.amount) as total_amount
    //         FROM proceedings p
    //         JOIN ledgers l ON p.db_to = l.id
    //         GROUP BY l.id, l.code, l.name
    //         ORDER BY total_amount DESC
    //         ",
    //         &[],
    //     )?;

    //     println!("\nSpending Report (by Outlay):");
    //     println!("{:<10} {:<30} {:<15}", "Code", "Name", "Total Amount");
    //     println!("{:-<55}", "");
    //     for row in rows {
    //         let code: String = row.get(0);
    //         let name: String = row.get(1);
    //         let total_amount: f64 = row.get(2);
    //         println!("{:<10} {:<30} {:<15.2}", code, name, total_amount);
    //     }
    //     Ok(())
    // }

    fn generate_spending_report(&mut self, period: ReportPeriod) -> Result<(), WalletError> {
        // Get the current time in UTC
        let now: DateTime<Utc> = Utc::now();

        // Calculate the start date based on the period
        let start_date: DateTime<Utc> = match period {
            ReportPeriod::Today => {
                // Start of today (00:00:00)
                now.with_hour(0)
                    .and_then(|d| d.with_minute(0))
                    .and_then(|d| d.with_second(0))
                    .and_then(|d| d.with_nanosecond(0))
                    .unwrap()
            }
            ReportPeriod::Week => {
                // Start of the week (Monday, 00:00:00)
                let days_since_monday = now.weekday().num_days_from_monday();
                now - Duration::days(days_since_monday as i64) + Duration::hours(0)
                    - Duration::minutes(now.minute() as i64)
                    - Duration::seconds(now.second() as i64)
                    - Duration::nanoseconds(now.nanosecond() as i64)
            }
            ReportPeriod::Month => {
                // Start of the month (1st day, 00:00:00)
                now.with_day(1)
                    .and_then(|d| d.with_hour(0))
                    .and_then(|d| d.with_minute(0))
                    .and_then(|d| d.with_second(0))
                    .and_then(|d| d.with_nanosecond(0))
                    .unwrap()
            }
            ReportPeriod::All => {
                // No filter (all time)
                DateTime::<Utc>::MIN_UTC
            }
        };

        let start_date_naive: NaiveDateTime = if period == ReportPeriod::All {
            // Use the earliest possible NaiveDateTime for "all" period
            NaiveDateTime::parse_from_str("1970-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")?
        } else {
            start_date.naive_utc() // Convert DateTime<Utc> to NaiveDateTime
        };

        // Prepare the query
        let query = if period == ReportPeriod::All {
            "
            SELECT l.code, l.name, SUM(p.amount) as total_amount
            FROM proceedings p
            JOIN ledgers l ON p.db_to = l.id
            GROUP BY l.id, l.code, l.name
            ORDER BY total_amount DESC
            "
        } else {
            "
            SELECT l.code, l.name, SUM(p.amount) as total_amount
            FROM proceedings p
            JOIN ledgers l ON p.db_to = l.id
            WHERE p.created_at >= $1
            GROUP BY l.id, l.code, l.name
            ORDER BY total_amount DESC
            "
        };

        // Execute the query
        let rows = if period == ReportPeriod::All {
            self.client.query(query, &[])?
        } else {
            self.client.query(query, &[&start_date_naive])?
        };

        // Print the report
        let period_str = match period {
            ReportPeriod::Today => "Today",
            ReportPeriod::Week => "This Week",
            ReportPeriod::Month => "This Month",
            ReportPeriod::All => "All Time",
        };

        println!("\nSpending Report ({}):", period_str);
        println!("{:<10} {:<30} {:<15}", "Code", "Name", "Total Amount");
        println!("{:-<55}", "");
        let mut grand_total: f64 = 0.0;

        for row in rows {
            let code: String = row.get(0);
            let name: String = row.get(1);
            let total_amount: f64 = row.get(2);
            grand_total += total_amount;
            println!("{:<10} {:<30} {:<15.2}", code, name, total_amount);
        }

        println!("{:-<55}", "");
        println!("{:<40} {:<15.2}", "Grand Total", grand_total);
        Ok(())
    }
    fn generate_spending_summary_report(
        &mut self,
        period: ReportPeriod,
    ) -> Result<(), WalletError> {
        let now: DateTime<Utc> = Utc::now();
        let start_date: DateTime<Utc> = match period {
            ReportPeriod::Today => now
                .with_hour(0)
                .and_then(|d| d.with_minute(0))
                .and_then(|d| d.with_second(0))
                .and_then(|d| d.with_nanosecond(0))
                .unwrap(),
            ReportPeriod::Week => {
                now - Duration::days(now.weekday().num_days_from_monday() as i64)
                    + Duration::hours(0)
                    - Duration::minutes(now.minute() as i64)
                    - Duration::seconds(now.second() as i64)
                    - Duration::nanoseconds(now.nanosecond() as i64)
            }
            ReportPeriod::Month => now
                .with_day(1)
                .and_then(|d| d.with_hour(0))
                .and_then(|d| d.with_minute(0))
                .and_then(|d| d.with_second(0))
                .and_then(|d| d.with_nanosecond(0))
                .unwrap(),
            ReportPeriod::All => DateTime::<Utc>::MIN_UTC,
        };

        let start_date_naive: NaiveDateTime = if period == ReportPeriod::All {
            NaiveDateTime::parse_from_str("1970-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")?
        } else {
            start_date.naive_utc()
        };

        // Query to calculate net amount (debits - credits) for each ledger
        let query = if period == ReportPeriod::All {
            "
            SELECT 
                l.code, 
                l.name, 
                COALESCE((
                    SELECT SUM(p1.amount) 
                    FROM proceedings p1 
                    WHERE p1.db_to = l.id
                ), 0) - COALESCE((
                    SELECT SUM(p2.amount) 
                    FROM proceedings p2 
                    WHERE p2.cr_from = l.id
                ), 0) as net_amount
            FROM ledgers l
            ORDER BY net_amount DESC
            "
        } else {
            "
            SELECT 
                l.code, 
                l.name, 
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
                ), 0) as net_amount
            FROM ledgers l
            ORDER BY net_amount DESC
            "
        };

        let rows = if period == ReportPeriod::All {
            self.client.query(query, &[])?
        } else {
            self.client.query(query, &[&start_date_naive])?
        };

        let period_str = match period {
            ReportPeriod::Today => "Today",
            ReportPeriod::Week => "This Week",
            ReportPeriod::Month => "This Month",
            ReportPeriod::All => "All Time",
        };

        println!("\nSpending Summary Report ({}):", period_str);
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
        // Retrieve the ledger ID
        let ledger_id = self.retrieve_ledger_id(ledger_code)?;

        // Get the ledger name for the report header
        let ledger_name: String = self
            .client
            .query_one("SELECT name FROM ledgers WHERE id = $1", &[&ledger_id])?
            .get(0);

        // Calculate the start date based on the period
        let now: DateTime<Utc> = Utc::now();
        let start_date: DateTime<Utc> = match period {
            ReportPeriod::Today => now
                .with_hour(0)
                .and_then(|d| d.with_minute(0))
                .and_then(|d| d.with_second(0))
                .and_then(|d| d.with_nanosecond(0))
                .unwrap(),
            ReportPeriod::Week => {
                now - Duration::days(now.weekday().num_days_from_monday() as i64)
                    + Duration::hours(0)
                    - Duration::minutes(now.minute() as i64)
                    - Duration::seconds(now.second() as i64)
                    - Duration::nanoseconds(now.nanosecond() as i64)
            }
            ReportPeriod::Month => now
                .with_day(1)
                .and_then(|d| d.with_hour(0))
                .and_then(|d| d.with_minute(0))
                .and_then(|d| d.with_second(0))
                .and_then(|d| d.with_nanosecond(0))
                .unwrap(),
            ReportPeriod::All => DateTime::<Utc>::MIN_UTC,
        };

        let start_date_naive: NaiveDateTime = if period == ReportPeriod::All {
            NaiveDateTime::parse_from_str("1970-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")?
        } else {
            start_date.naive_utc()
        };
        let query = if period == ReportPeriod::All {
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
        } else {
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
        };
        // Execute the query
        let rows = if period == ReportPeriod::All {
            self.client.query(query, &[&ledger_id])?
        } else {
            self.client.query(query, &[&ledger_id, &start_date_naive])?
        };

        // Print the report
        let period_str = match period {
            ReportPeriod::Today => "Today",
            ReportPeriod::Week => "This Week",
            ReportPeriod::Month => "This Month",
            ReportPeriod::All => "All Time",
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

        // Print totals and net balance
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
        #[arg(value_enum, default_value_t = ReportPeriod::All)]
        period: ReportPeriod,
    },
    SummaryReport {
        #[arg(value_enum, default_value_t = ReportPeriod::All)]
        period: ReportPeriod,
    },
    LedgerReport {
        code: String,
        #[arg(value_enum, default_value_t = ReportPeriod::All)]
        period: ReportPeriod,
    },
    /// List all ledgers
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
        Commands::Report { period } => {
            db.generate_spending_report(period).map_err(|e| {
                eprintln!("Failed to generate report: {}", e);
                e
            })?;
        }
        Commands::SummaryReport { period } => {
            db.generate_spending_summary_report(period).map_err(|e| {
                eprintln!("Failed to generate report: {}", e);
                e
            })?;
        }
        Commands::LedgerReport { code, period } => {
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
