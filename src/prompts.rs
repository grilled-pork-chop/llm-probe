//! Built-in prompt pool derived from ShareGPT first-turns.
//!
//! Used as the default turn source when --turn is not specified, giving
//! realistic, cache-busting traffic across diverse topics and lengths.

use rand::SeedableRng;
use rand::rngs::SmallRng;
use rand::seq::IndexedRandom;

pub struct PromptSampler {
    rng: SmallRng,
    last: Option<usize>,
}

impl PromptSampler {
    pub fn new() -> Self {
        Self { rng: SmallRng::from_os_rng(), last: None }
    }

    /// Pick a prompt at random, avoiding an immediate repeat.
    pub fn next(&mut self) -> &'static str {
        let len = PROMPTS.len();
        loop {
            let idx = (0..len).collect::<Vec<_>>().choose(&mut self.rng).copied().unwrap_or(0);
            if Some(idx) != self.last {
                self.last = Some(idx);
                return PROMPTS[idx];
            }
        }
    }
}

pub static PROMPTS: &[&str] = &[
    // Coding
    "Write a Python function that finds all prime numbers up to n using the Sieve of Eratosthenes.",
    "Explain the difference between TCP and UDP with a practical example.",
    "How do I reverse a linked list in place in C++?",
    "Write a SQL query to find the top 5 customers by total order value.",
    "What is the difference between a process and a thread?",
    "Write a regex that matches valid email addresses.",
    "Implement a binary search algorithm in Rust.",
    "What are the SOLID principles and why do they matter?",
    "How does garbage collection work in Java?",
    "Write a bash script that monitors disk usage and emails an alert when it exceeds 80%.",
    "Explain how a hash table works, including collision handling strategies.",
    "What is the time complexity of quicksort in the average and worst case?",
    "Write a React component that fetches and displays a list of users from an API.",
    "How do I set up a Python virtual environment and why should I use one?",
    "Explain the difference between `==` and `===` in JavaScript.",
    "Write a Dockerfile for a Node.js application.",
    "What is a deadlock and how can it be prevented?",
    "How does async/await work under the hood in Python?",
    "Write a function to detect if a string is a palindrome.",
    "Explain the CAP theorem in distributed systems.",
    "How does the Linux kernel handle system calls?",
    "What is dependency injection and how does it improve testability?",
    "Write a Python script to parse a CSV file and compute column averages.",
    "How does HTTPS work? Explain the TLS handshake.",
    "What is the difference between REST and GraphQL?",
    "Implement a stack using two queues.",
    "What are Python decorators and how do you write one?",
    "How does Git's internal object model work?",
    "Write a function that detects cycles in a directed graph.",
    "What is memoization and when should you use it?",
    "How do database indexes work and what are the trade-offs?",
    "Write a WebSocket server in Python using asyncio.",
    "Explain the difference between horizontal and vertical scaling.",
    "What is a race condition? Give a concrete example.",
    "How do I implement rate limiting in an API?",
    "Write a merge sort implementation in Go.",
    "What is the difference between a mutex and a semaphore?",
    "How does Docker networking work?",
    "Explain the event loop in Node.js.",
    "Write a function to serialize and deserialize a binary tree.",

    // Writing & language
    "Write a cover letter for a software engineer applying to a fintech startup.",
    "Summarize the main arguments for and against universal basic income.",
    "Write a short story about a lighthouse keeper who discovers something unexpected.",
    "How do I improve the clarity and conciseness of my writing?",
    "Write a professional email declining a meeting invitation.",
    "Explain the difference between active and passive voice with examples.",
    "Write an introduction paragraph for an essay about climate change.",
    "What makes a great TED talk? Break down the key elements.",
    "Rewrite this sentence to be more formal: 'We gotta fix this ASAP or things will go south.'",
    "Write a product description for a pair of noise-cancelling headphones.",
    "What are the most common grammar mistakes in English writing?",
    "Write a persuasive argument for why cities should ban single-use plastics.",
    "How do I structure a technical blog post that developers will actually read?",
    "Write a haiku about debugging code at 2am.",
    "Explain the hero's journey narrative structure with a modern film example.",

    // Math & reasoning
    "Explain the Monty Hall problem and why switching doors increases your odds.",
    "What is Bayes' theorem and how is it used in spam filtering?",
    "Prove that there are infinitely many prime numbers.",
    "How do you calculate the probability of getting at least one six in four dice rolls?",
    "Explain what a derivative is intuitively, without using formulas first.",
    "What is the difference between correlation and causation? Give an example.",
    "How does public-key cryptography work mathematically?",
    "What is the birthday paradox and what is the probability for a group of 23 people?",
    "Explain gradient descent intuitively.",
    "What is Big O notation and why does it matter for software engineers?",
    "Solve: if a train leaves Chicago at 60 mph and another leaves New York at 80 mph, when do they meet?",
    "What is the difference between mean, median, and mode and when should you use each?",
    "Explain Euler's identity and why mathematicians find it beautiful.",
    "What is a Fourier transform and what is it used for?",
    "How does RSA encryption work step by step?",

    // Science & technology
    "How does a neural network learn? Explain backpropagation simply.",
    "What is the difference between supervised and unsupervised machine learning?",
    "Explain how CRISPR gene editing works.",
    "What causes a solar eclipse and how often does one occur?",
    "How does a lithium-ion battery work at the chemical level?",
    "What is quantum entanglement and does it allow faster-than-light communication?",
    "Explain the difference between RAM and storage, and why both matter.",
    "How do vaccines train the immune system?",
    "What is a black hole and what happens at the event horizon?",
    "Explain how GPS determines your location.",
    "What is the difference between fusion and fission?",
    "How does a computer processor execute machine instructions?",
    "What causes the northern lights?",
    "Explain how transformer models work at a high level.",
    "What is the difference between a virus and a bacterium?",
    "How does noise-cancelling technology work?",
    "What is CERN doing and why does it matter?",
    "Explain Moore's Law and whether it still holds today.",
    "How do recommendation systems like Netflix's work?",
    "What is the difference between IPv4 and IPv6?",

    // History & society
    "What were the main causes of World War I?",
    "Explain the significance of the Magna Carta.",
    "How did the printing press change European society?",
    "What was the Cold War and what ended it?",
    "Explain the differences between capitalism, socialism, and communism.",
    "What caused the 2008 financial crisis?",
    "How did the Roman Empire fall?",
    "What is the significance of the Marshall Plan?",
    "Explain the origins of the internet.",
    "What were the main drivers of the Industrial Revolution?",
    "How did colonialism shape modern Africa?",
    "What is the difference between a democracy and a republic?",
    "Explain the significance of the moon landing in 1969.",
    "What caused the Great Depression and how did it end?",
    "How did the Silk Road shape world trade and culture?",

    // Philosophy & ethics
    "What is the trolley problem and what does it reveal about ethics?",
    "Explain the difference between deontological and consequentialist ethics.",
    "What did Socrates mean by 'the unexamined life is not worth living'?",
    "Is free will compatible with determinism? Explain the main positions.",
    "What is Occam's razor and when should you apply it?",
    "Explain Plato's allegory of the cave.",
    "What are the ethical implications of artificial general intelligence?",
    "What is the ship of Theseus paradox?",
    "Should self-driving cars be programmed to minimize passenger or pedestrian harm in unavoidable crashes?",
    "What is existentialism and who are its key thinkers?",

    // Practical & everyday
    "What is the best way to negotiate a salary increase?",
    "How do I start investing with a small amount of money?",
    "What are the most effective study techniques backed by research?",
    "How does compound interest work and why does it matter for retirement?",
    "What are the key differences between renting and buying a home?",
    "How do I build a habit and make it stick?",
    "What should I know before starting my first business?",
    "How does sleep affect cognitive performance?",
    "What are the most evidence-based ways to reduce stress?",
    "How do I read a research paper effectively?",
    "What is the best way to give constructive feedback?",
    "How should I structure my day for maximum productivity?",
    "What is the difference between a will and a living trust?",
    "How do I evaluate whether a news article is reliable?",
    "What are the basics of personal finance I should know by 30?",

    // Data & analysis
    "What is the difference between a data warehouse and a data lake?",
    "Explain the ETL process and why it matters.",
    "How do I choose between a bar chart and a line chart?",
    "What is A/B testing and how do you design a valid experiment?",
    "Explain what p-value means in simple terms.",
    "What is overfitting in machine learning and how do you prevent it?",
    "How does k-means clustering work?",
    "What is the difference between precision and recall?",
    "How do I handle missing data in a dataset?",
    "What is a confusion matrix and how do I interpret it?",

    // Creative & fun
    "If you could have dinner with any three historical figures, who would you choose and why?",
    "Write a plot synopsis for a sci-fi novel where AI achieves consciousness inside a submarine.",
    "What would a city designed entirely around pedestrians and cyclists look like?",
    "Invent a new sport that combines chess and a physical activity.",
    "If the internet disappeared tomorrow, what would change most about daily life?",
    "Write a dialogue between Newton and Einstein discussing gravity.",
    "What would it take to terraform Mars for human habitation?",
    "Design a school curriculum optimized for the 21st century.",
    "If you could eliminate one human cognitive bias, which would have the most positive impact?",
    "What would global governance look like in 200 years?",

    // DevOps & infrastructure
    "Explain the difference between blue-green and canary deployments.",
    "How does Kubernetes decide where to schedule a pod?",
    "What is infrastructure as code and what are its main benefits?",
    "How do I design a CI/CD pipeline for a microservices application?",
    "What is the difference between a load balancer and an API gateway?",
    "Explain how Prometheus and Grafana work together for monitoring.",
    "What are the trade-offs between monolithic and microservices architecture?",
    "How does service mesh like Istio improve security in Kubernetes?",
    "What is chaos engineering and why do companies like Netflix use it?",
    "How do I estimate the cost of running a workload on AWS?",

    // Security
    "What is SQL injection and how do I prevent it?",
    "Explain how a man-in-the-middle attack works.",
    "What is the difference between authentication and authorization?",
    "How does OAuth 2.0 work?",
    "What is a zero-day vulnerability?",
    "Explain the principle of least privilege.",
    "How does a VPN protect your privacy?",
    "What is cross-site scripting (XSS) and how do I defend against it?",
    "What is the difference between symmetric and asymmetric encryption?",
    "How should I store passwords securely in a database?",
];
