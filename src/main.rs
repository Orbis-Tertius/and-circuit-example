use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{AssignedCell, Chip, Layouter, Region, SimpleFloorPlanner},
    dev::MockProver,
    plonk::{
        Advice, Circuit, Column, ConstraintSystem, Error, Expression, Fixed, Instance, Selector,
        TableColumn,
    },
    poly::Rotation,
};
use pasta_curves::{group::ff::PrimeField, Fp};
use std::marker::PhantomData;

const WORD_BITS: u32 = 8;

pub trait NumericInstructions<F: FieldExt>: Chip<F> {
    /// Variable representing a number.
    type Word;

    /// Loads a number into the circuit as a private input.
    fn load_private(&self, layouter: impl Layouter<F>, a: Option<F>) -> Result<Self::Word, Error>;

    fn add(
        &self,
        layouter: impl Layouter<F>,
        a: Self::Word,
        b: Self::Word,
    ) -> Result<Self::Word, Error>;

    fn verify_decompose(
        &self,
        layouter: impl Layouter<F>,
        c: Self::Word,
    ) -> Result<(Self::Word, Self::Word), Error>;

    fn compose(
        &self,
        layouter: impl Layouter<F>,
        a: Self::Word,
        b: Self::Word,
    ) -> Result<Self::Word, Error>;

    /// Exposes a number as a public input to the circuit.
    fn expose_public(
        &self,
        layouter: impl Layouter<F>,
        num: Self::Word,
        row: usize,
    ) -> Result<(), Error>;
}

/// The chip that will implement our instructions! Chips store their own
/// config, as well as type markers if necessary.
pub struct AndChip<F: FieldExt> {
    config: AndConfig,
    _marker: PhantomData<F>,
}

/// Chip state is stored in a config struct. This is generated by the chip
/// during configuration, and then stored inside the chip.
#[derive(Clone, Debug)]
pub struct AndConfig {
    /// For this chip, we will use two advice columns to implement our instructions.
    /// These are also the columns through which we communicate with other parts of
    /// the circuit.
    advice: [Column<Advice>; 2],

    /// This is the public input (instance) column.
    instance: Column<Instance>,

    even_bits: TableColumn,

    // We need a selector to enable the add gate, so that we aren't placing
    // any constraints on cells where `NumericInstructions::add` is not being used.
    // This is important when building larger circuits, where columns are used by
    // multiple sets of instructions.
    s_add: Selector,
    s_decompose: Selector,
    s_compose: Selector,
}

impl<F: FieldExt> AndChip<F> {
    fn construct(config: <Self as Chip<F>>::Config) -> Self {
        Self {
            config,
            _marker: PhantomData,
        }
    }

    fn configure(
        meta: &mut ConstraintSystem<F>,
        advice: [Column<Advice>; 2],
        instance: Column<Instance>,
        constant: Column<Fixed>,
    ) -> <Self as Chip<F>>::Config {
        meta.enable_equality(instance);
        meta.enable_constant(constant);
        for column in &advice {
            meta.enable_equality(*column);
        }
        let s_add = meta.selector();
        let s_decompose = meta.complex_selector();
        let s_compose = meta.selector();
        let even_bits = meta.lookup_table_column();

        meta.create_gate("add", |meta| {
            let lhs = meta.query_advice(advice[0], Rotation::cur());
            let rhs = meta.query_advice(advice[1], Rotation::cur());
            let out = meta.query_advice(advice[0], Rotation::next());
            let s_add = meta.query_selector(s_add);

            // Finally, we return the polynomial expressions that constrain this gate.
            // For our multiplication gate, we only need a single polynomial constraint.
            //
            // The polynomial expressions returned from `create_gate` will be
            // constrained by the proving system to equal zero. Our expression
            vec![s_add * (lhs + rhs - out)]
        });

        meta.create_gate("decompose", |meta| {
            let lhs = dbg!(meta.query_advice(advice[0], Rotation::cur()));
            let rhs = dbg!(meta.query_advice(advice[1], Rotation::cur()));
            let out = dbg!(meta.query_advice(advice[0], Rotation::next()));
            let s_decompose = meta.query_selector(s_decompose);

            // Finally, we return the polynomial expressions that constrain this gate.
            // For our multiplication gate, we only need a single polynomial constraint.
            //
            // The polynomial expressions returned from `create_gate` will be
            // constrained by the proving system to equal zero. Our expression
            vec![s_decompose * (lhs + Expression::Constant(F::from(2)) * rhs - out)]
        });

        meta.create_gate("compose", |meta| {
            let lhs = meta.query_advice(advice[0], Rotation::cur());
            let rhs = meta.query_advice(advice[1], Rotation::cur());
            let out = meta.query_advice(advice[0], Rotation::next());
            let s_compose = meta.query_selector(s_compose);

            // Finally, we return the polynomial expressions that constrain this gate.
            // For our multiplication gate, we only need a single polynomial constraint.
            //
            // The polynomial expressions returned from `create_gate` will be
            // constrained by the proving system to equal zero. Our expression
            vec![s_compose * (lhs + Expression::Constant(F::from(2)) * rhs - out)]
        });

        let _ = meta.lookup(|meta| {
            let lookup = meta.query_selector(s_decompose);
            let a = meta.query_advice(advice[0], Rotation::cur());

            vec![(lookup * a, even_bits)]
        });

        let _ = meta.lookup(|meta| {
            let lookup = meta.query_selector(s_decompose);
            let b = meta.query_advice(advice[1], Rotation::cur());

            vec![(lookup * b, even_bits)]
        });

        AndConfig {
            advice,
            instance,
            even_bits,
            s_add,
            s_decompose,
            s_compose,
        }
    }

    // Allocates all even bits in a a table for the word size AND_BITS.
    // `2^(WORD_BITS/2)` rows of the constraint system.
    fn alloc_table(&self, layouter: &mut impl Layouter<Fp>) -> Result<(), Error> {
        layouter.assign_table(
            || "even bits table",
            |mut table| {
                for i in 0..2usize.pow(WORD_BITS / 2) {
                    table.assign_cell(
                        || format!("even_bits row {}", i),
                        self.config.even_bits,
                        i,
                        || Ok(Fp::from(even_bits_at(i) as u64)),
                    )?;
                }
                Ok(())
            },
        )
    }
}

fn even_bits_at(mut i: usize) -> usize {
    let mut r = 0;
    let mut c = 0;

    while i != 0 {
        let lower_bit = i % 2;
        r += lower_bit * 4usize.pow(c);
        i >>= 1;
        c += 1;
    }

    eprintln!("{:#08b}", r);
    r
}

#[test]
fn even_bits_at_test() {
    assert_eq!(0b0, even_bits_at(0));
    assert_eq!(0b1, even_bits_at(1));
    assert_eq!(0b100, even_bits_at(2));
    assert_eq!(0b101, even_bits_at(3));
}

impl<F: FieldExt> Chip<F> for AndChip<F> {
    type Config = AndConfig;
    type Loaded = ();

    fn config(&self) -> &Self::Config {
        &self.config
    }

    fn loaded(&self) -> &Self::Loaded {
        &()
    }
}

/// A variable representing a number.
#[derive(Clone, Debug)]
pub struct Word<F: FieldExt>(AssignedCell<F, F>);

impl NumericInstructions<Fp> for AndChip<Fp> {
    type Word = Word<Fp>;

    fn load_private(
        &self,
        mut layouter: impl Layouter<Fp>,
        value: Option<Fp>,
    ) -> Result<Self::Word, Error> {
        let config = self.config();

        layouter.assign_region(
            || "load private",
            |mut region| {
                region
                    .assign_advice(
                        || "private input",
                        config.advice[0],
                        0,
                        || value.ok_or(Error::Synthesis),
                    )
                    .map(Word)
            },
        )
    }

    fn add(
        &self,
        mut layouter: impl Layouter<Fp>,
        a: Self::Word,
        b: Self::Word,
    ) -> Result<Self::Word, Error> {
        let config = self.config();

        layouter.assign_region(
            || "add",
            |mut region: Region<'_, Fp>| {
                // We only want to use a single addition gate in this region,
                // so we enable it at region offset 0; this means it will constrain
                // cells at offsets 0 and 1.
                config.s_add.enable(&mut region, 0)?;

                // The inputs we've been given could be located anywhere in the circuit,
                // but we can only rely on relative offsets inside this region. So we
                // assign new cells inside the region and constrain them to have the
                // same values as the inputs.
                a.0.copy_advice(|| "lhs", &mut region, config.advice[0], 0)?;
                b.0.copy_advice(|| "rhs", &mut region, config.advice[1], 0)?;

                // Now we can assign the addition result, which is to be assigned
                // into the output position.
                let value = a.0.value().and_then(|a| b.0.value().map(|b| *a + *b));

                // Finally, we do the assignment to the output, returning a
                // variable to be used in another part of the circuit.
                region
                    .assign_advice(
                        || "lhs + rhs",
                        config.advice[0],
                        1,
                        || value.ok_or(Error::Synthesis),
                    )
                    .map(Word)
            },
        )
    }

    fn verify_decompose(
        &self,
        mut layouter: impl Layouter<Fp>,
        c: Self::Word,
    ) -> Result<(Self::Word, Self::Word), Error> {
        let config = self.config();

        layouter.assign_region(
            || "decompose",
            |mut region: Region<'_, Fp>| {
                // We only want to use a single addition gate in this region,
                // so we enable it at region offset 0; this means it will constrain
                // cells at offsets 0 and 1.
                config.s_decompose.enable(&mut region, 0)?;

                let o_oe = c.0.value().map(|c| decompose(*c));
                let e_cell = region
                    .assign_advice(
                        || "even bits",
                        config.advice[0],
                        0,
                        || o_oe.map(|oe| oe.0).ok_or(Error::Synthesis),
                    )
                    .map(Word)?;

                let o_cell = region
                    .assign_advice(
                        || "odd bits",
                        config.advice[1],
                        0,
                        || o_oe.map(|oe| dbg!(oe.1)).ok_or(Error::Synthesis),
                    )
                    .map(Word)?;

                // The inputs we've been given could be located anywhere in the circuit,
                // but we can only rely on relative offsets inside this region. So we
                // assign new cells inside the region and constrain them to have the
                // same values as the inputs.
                c.0.copy_advice(|| "out", &mut region, config.advice[0], 1)?;
                Ok((e_cell, o_cell))
            },
        )
    }

    fn compose(
        &self,
        mut layouter: impl Layouter<Fp>,
        a: Self::Word,
        b: Self::Word,
    ) -> Result<Self::Word, Error> {
        let config = self.config();

        layouter.assign_region(
            || "compose",
            |mut region: Region<'_, Fp>| {
                config.s_compose.enable(&mut region, 0)?;
                a.0.copy_advice(|| "lhs", &mut region, config.advice[0], 0)?;
                b.0.copy_advice(|| "rhs", &mut region, config.advice[1], 0)?;
                let value =
                    a.0.value()
                        .and_then(|a| b.0.value().map(|b| *a + Fp::from(2) * *b));

                region
                    .assign_advice(
                        || "lhs + rhs",
                        config.advice[0],
                        1,
                        || value.ok_or(Error::Synthesis),
                    )
                    .map(Word)
            },
        )
    }

    fn expose_public(
        &self,
        mut layouter: impl Layouter<Fp>,
        num: Self::Word,
        row: usize,
    ) -> Result<(), Error> {
        let config = self.config();

        layouter.constrain_instance(num.0.cell(), config.instance, row)
    }
}

/// The full circuit implementation.
///
/// In this struct we store the private input variables. We use `Option<F>` because
/// they won't have any value during key generation. During proving, if any of these
/// were `None` we would get an error.
#[derive(Default)]
pub struct MyCircuit<F: FieldExt> {
    pub a: Option<F>,
    pub b: Option<F>,
}

// impl<F: FieldExt> Circuit<F> for MyCircuit<F> {
impl Circuit<Fp> for MyCircuit<Fp> {
    // Since we are using a single chip for everything, we can just reuse its config.
    type Config = AndConfig;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        Self::default()
    }

    // fn configure(meta: &mut ConstraintSystem<F>) -> Self::Config {
    fn configure(meta: &mut ConstraintSystem<Fp>) -> Self::Config {
        // We create the two advice columns that FieldChip uses for I/O.
        let advice = [meta.advice_column(), meta.advice_column()];

        // We also need an instance column to store public inputs.
        let instance = meta.instance_column();

        // Create a fixed column to load constants.
        let constant = meta.fixed_column();

        AndChip::configure(meta, advice, instance, constant)
    }

    fn synthesize(
        &self,
        config: Self::Config,
        // mut layouter: impl Layouter<F>,
        mut layouter: impl Layouter<Fp>,
    ) -> Result<(), Error> {
        // let field_chip = AndChip::<F>::construct(config);
        let field_chip = AndChip::<Fp>::construct(config);
        field_chip.alloc_table(&mut layouter.namespace(|| "alloc table"))?;

        // Load our private values into the circuit.
        // index 0
        let a = field_chip.load_private(layouter.namespace(|| "load a"), self.a)?;
        // index 1
        let b = field_chip.load_private(layouter.namespace(|| "load b"), self.b)?;

        // index 2
        let (ae, ao) =
            field_chip.verify_decompose(layouter.namespace(|| "a decomposition"), dbg!(a))?;

        // index 3
        let (be, bo) =
            field_chip.verify_decompose(layouter.namespace(|| "b decomposition"), dbg!(b))?;

        // index 4
        let e = field_chip.add(layouter.namespace(|| "ae + be"), ae, be)?;
        // index 5
        let o = field_chip.add(layouter.namespace(|| "ao + be"), ao, bo)?;

        // // index 6
        let (_ee, eo) =
            field_chip.verify_decompose(layouter.namespace(|| "e decomposition"), dbg!(e))?;

        // index 7
        let (_oe, oo) =
            field_chip.verify_decompose(layouter.namespace(|| "o decomposition"), dbg!(o))?;

        // // index 8
        let a_and_b = field_chip.compose(
            layouter.namespace(|| "compose eo and oo"),
            dbg!(eo),
            dbg!(oo),
        )?;

        // Expose the result as a public input to the circuit.
        // field_chip.expose_public(layouter.namespace(|| "expose a_and_b"), dbg!(a_and_b), 0)
        field_chip.expose_public(layouter.namespace(|| "expose a_and_b"), dbg!(a_and_b), 0)
    }
}

/// Rtc/fonts/conf.d/40-nonlatin.conf", line 4: unknown element "description"
fn decompose(word: Fp) -> (Fp, Fp) {
    let mut even_only = word.to_repr();
    even_only.iter_mut().for_each(|bits| {
        *bits &= 0b01010101;
    });

    let mut odd_only = word.to_repr();
    odd_only.iter_mut().for_each(|bits| {
        *bits &= 0b10101010;
    });

    let even_only = Fp::from_repr(even_only).unwrap();
    let odd_only = Fp::from_repr(odd_only).unwrap();

    (even_only, Fp::from_u128(odd_only.get_lower_128() >> 1))
}

#[test]
fn decompose_test_even_odd() {
    let odds = 0xAAAA;
    let evens = 0x5555;
    let (e, o) = decompose(Fp::from_u128(odds));
    assert_eq!(e.get_lower_128(), 0);
    assert_eq!(o.get_lower_128(), odds >> 1);
    let (e, o) = decompose(Fp::from_u128(evens));
    assert_eq!(e.get_lower_128(), evens);
    assert_eq!(o.get_lower_128(), 0);
}

use proptest::prelude::*;
proptest! {
    #[test]
    fn decompose_test(a in 0..u128::MAX) {
        let a = Fp::from_u128(a);
        decompose(a);
    }

    #[test]
    fn fp_u128_test(n in 0..u128::MAX) {
        let a = Fp::from_u128(n);
        let b = a.get_lower_128();
        assert_eq!(b, n)
    }

    #[test]
    fn all_words_test(a in 0..2u64.pow(WORD_BITS), b in 0..2u64.pow(WORD_BITS)) {
      let k = 5;
      let circuit = MyCircuit {
          a: Some(Fp::from(a)),
          b: Some(Fp::from(b)),
      };

      let c = Fp::from(a & b);

      // Arrange the public input. We expose the bitwise AND result in row 0
      // of the instance column, so we position it there in our public inputs.
      let public_inputs = vec![c];

      // Given the correct public input, our circuit will verify.
      let prover = MockProver::run(k, &circuit, vec![public_inputs]).unwrap();
      assert_eq!(prover.verify(), Ok(()));
    }
}

#[test]
fn circuit_layout_test() {
    let k = 5;

    // Prepare the private and public inputs to the circuit!
    const A: u64 = 7;
    const B: u64 = 6;
    let a = Fp::from(A);
    let b = Fp::from(B);

    // Instantiate the circuit with the private inputs.
    let circuit = MyCircuit {
        a: Some(a),
        b: Some(b),
    };
    use plotters::prelude::*;
    let root = BitMapBackend::new("layout.png", (1920, 1080)).into_drawing_area();
    root.fill(&WHITE).unwrap();
    let root = root
        .titled("Bitwise AND Circuit Layout", ("sans-serif", 60))
        .unwrap();

    halo2_proofs::dev::CircuitLayout::default()
        .mark_equality_cells(true)
        .show_equality_constraints(true)
        // The first argument is the size parameter for the circuit.
        .render(k, &circuit, &root)
        .unwrap();

    let dot_string = halo2_proofs::dev::circuit_dot_graph(&circuit);
    let mut dot_graph = std::fs::File::create("circuit.dot").unwrap();
    std::io::Write::write_all(&mut dot_graph, dot_string.as_bytes()).unwrap();
}

fn main() {
    // ANCHOR: test-circuit
    // The number of rows in our circuit cannot exceed 2^k. Since our example
    // circuit is very small, we can pick a very small value here.
    let k = 5;

    // Prepare the private and public inputs to the circuit!
    const A: u64 = 3;
    const B: u64 = 4;
    let a = Fp::from(A);
    let b = Fp::from(B);
    let c = Fp::from(A & B);

    let (a_e, a_o) = decompose(a);
    eprintln!("a_e: {:#08b}", &a_e.get_lower_128());
    eprintln!("a_o: {:#08b}", &a_o.get_lower_128());
    eprintln!("c:   {:#08b}", &a_o.get_lower_128());

    // Instantiate the circuit with the private inputs.
    let circuit = MyCircuit {
        a: Some(a),
        b: Some(b),
    };

    // Arrange the public input. We expose the bitwise AND result in row 0
    // of the instance column, so we position it there in our public inputs.
    let public_inputs = vec![c];

    // Given the correct public input, our circuit will verify.
    let prover = MockProver::run(k, &circuit, vec![public_inputs]).unwrap();
    assert_eq!(prover.verify(), Ok(()));
}
