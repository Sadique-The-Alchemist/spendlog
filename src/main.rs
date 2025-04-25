use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use postgres::{Client, Error as PgError, NoTls};
// WalletDB struct to manage database connection
struct WalletDB {
    client: Client,
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
    fn new() -> Result<Self, PgError> {
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
    ) -> Result<(), PgError> {
        self.client.execute(
            "INSERT INTO ledgers (code, name, description, sort, kind) VALUES ($1, $2, $3, $4, $5)",
            &[&code, &name, &description, &sort, &kind],
        )?;
        println!("Added ledger: {} - {}", code, name);
        Ok(())
    }

    fn retrieve_ledger_id(&mut self, code: &str) -> Result<i32, PgError> {
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
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Validate amount
        if amount <= 0.0 {
            return Err("Amount must be positive".into());
        }

        // Retrieve ledger IDs
        let patron_id = self.retrieve_ledger_id(patron)?;
        let outlay_id = self.retrieve_ledger_id(outlay)?;

        // Insert the spending entry
        self.client.execute(
            "INSERT INTO proceedings (cr_from, db_to, amount, narration) VALUES ($1, $2, $3, $4)",
            &[&patron_id, &outlay_id, &amount, &narration],
        )?;
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

    fn generate_spending_report(&mut self, period: ReportPeriod) -> Result<(), PgError> {
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
            self.client.query(query, &[&start_date])?
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
        for row in rows {
            let code: String = row.get(0);
            let name: String = row.get(1);
            let total_amount: f64 = row.get(2);
            println!("{:<10} {:<30} {:<15.2}", code, name, total_amount);
        }
        Ok(())
    }

    // New method to list all ledgers (helpful for debugging or user reference)
    fn list_ledgers(&mut self) -> Result<(), PgError> {
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

    fn setup_db(&mut self) -> Result<(), PgError> {
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
    fn clear_database(&mut self) -> Result<(), PgError> {
        self.client.execute("DELETE FROM proceedings", &[])?;
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
    },
    /// Generate a spending report
    Report {
        #[arg(long, value_enum, default_value_t = ReportPeriod::All)]
        period: ReportPeriod,
    },
    /// List all ledgers
    ListLedgers,
    DbSetup,
    Clear,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
            db.add_ledger(&code, &name, &description, &sort, &kind)?;
        }
        Commands::Spend {
            patron,
            outlay,
            amount,
            narration,
        } => {
            db.proceed_spend(&patron, &outlay, amount, &narration)?;
        }
        Commands::Report { period } => {
            db.generate_spending_report(period)?;
        }
        Commands::ListLedgers => {
            db.list_ledgers()?;
        }
        Commands::DbSetup => {
            db.setup_db()?;
        }
        Commands::Clear => {
            db.clear_database()?;
        }
    }

    Ok(())
}
