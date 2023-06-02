use std::collections::HashMap;

use crate::align::aligners::constants::DEFAULT_ALIGNER_CAPACITY;
use crate::align::aligners::single_contig_aligner::SingleContigAligner;
use crate::align::alignment::Alignment;
use crate::align::scoring::Scoring;
use crate::align::traceback::traceback;
use bio::alignment::pairwise::MatchFunc;
use bio::utils::TextSlice;
use itertools::Itertools;

use super::JumpInfo;

struct ContigAligner<'a, F: MatchFunc> {
    pub name: String,
    pub is_forward: bool,
    pub aligner: SingleContigAligner<F>,
    pub seq: &'a [u8],
}

impl<'a, F: MatchFunc> ContigAligner<'a, F> {
    pub fn new(
        name: String,
        is_forward: bool,
        scoring: Scoring<F>,
        seq: TextSlice<'a>,
        contig_idx: usize,
        circular: bool,
    ) -> ContigAligner<'a, F> {
        let mut aligner = SingleContigAligner::with_capacity_and_scoring(
            DEFAULT_ALIGNER_CAPACITY,
            DEFAULT_ALIGNER_CAPACITY,
            scoring,
        );
        aligner.set_contig_idx(contig_idx);
        aligner.set_circular(circular);
        Self {
            name,
            is_forward,
            aligner,
            seq,
        }
    }

    pub fn len(&self) -> usize {
        self.seq.len()
    }
}

pub struct MultiContigAligner<'a, F: MatchFunc> {
    contigs: Vec<ContigAligner<'a, F>>,
    name_to_forward: HashMap<String, usize>,
    name_to_revcomp: HashMap<String, usize>,
}

impl<'a, F: MatchFunc> MultiContigAligner<'a, F> {
    /// Create new aligner instance with given scorer.
    ///
    /// # Arguments
    ///
    /// * `scoring_fn` - function that returns an alignment scorer
    ///    (see also [`bio::alignment::pairwise::Scoring`](struct.Scoring.html))
    pub fn new() -> Self {
        MultiContigAligner {
            contigs: Vec::new(),
            name_to_forward: HashMap::new(),
            name_to_revcomp: HashMap::new(),
        }
    }

    fn hashmap_for_strand_mut(&mut self, is_forward: bool) -> &mut HashMap<String, usize> {
        if is_forward {
            &mut self.name_to_forward
        } else {
            &mut self.name_to_revcomp
        }
    }

    fn hashmap_for_strand(&self, is_forward: bool) -> &HashMap<String, usize> {
        if is_forward {
            &self.name_to_forward
        } else {
            &self.name_to_revcomp
        }
    }

    /// Adds a new aligner for the given contig and strand.
    pub fn add_contig(
        &mut self,
        name: &str,
        is_forward: bool,
        seq: TextSlice<'a>,
        circular: bool,
        scoring: Scoring<F>,
    ) {
        assert!(
            !self.hashmap_for_strand(is_forward).contains_key(name),
            "Contig already added! name: {name} is_forward: {is_forward}"
        );

        let contig_idx: usize = self.contigs.len();
        let contig_aligner = ContigAligner::new(
            name.to_string(),
            is_forward,
            scoring,
            seq,
            contig_idx,
            circular,
        );
        self.contigs.push(contig_aligner);
        self.hashmap_for_strand_mut(is_forward)
            .insert(name.to_string(), contig_idx);
    }

    fn jump_info_for_contig(contig: &ContigAligner<'a, F>, j: usize) -> JumpInfo {
        contig.aligner.get_jump_info(
            contig.len(),
            j - 1,
            contig.aligner.scoring.jump_score_same_contig_and_strand,
        )
    }

    fn jump_info_for_opposite_strand(
        &self,
        opp_contig_idx: Option<usize>,
        j: usize,
    ) -> Option<JumpInfo> {
        opp_contig_idx.map(|idx| {
            let opp = &self.contigs[idx];
            let mut info = opp.aligner.get_jump_info(
                opp.len(),
                j - 1,
                opp.aligner.scoring.jump_score_same_contig_opposite_strand,
            );
            info.idx = opp.aligner.contig_idx;
            info
        })
    }

    fn jump_info_for_inter_contig(
        contig: &ContigAligner<'a, F>,
        inter_contig_jump_infos: &[JumpInfo],
        opp_contig_idx: Option<usize>,
    ) -> Option<JumpInfo> {
        let opp_contig_idx = opp_contig_idx.map_or(contig.aligner.contig_idx, |idx| idx as u32);
        inter_contig_jump_infos
            .iter()
            .filter(|info| info.idx != contig.aligner.contig_idx && info.idx != opp_contig_idx)
            .max_by_key(|c| (c.score, c.len))
            .copied()
    }

    /// The core function to compute the alignment
    ///
    /// # Arguments
    ///
    /// * `x` - Textslice
    /// * `y` - Textslice
    pub fn custom(&mut self, y: TextSlice<'_>) -> Alignment {
        let n = y.len();

        // Set the initial conditions
        // We are repeating some work, but that's okay!
        for contig in &mut self.contigs {
            contig.aligner.init_matrices(contig.len(), n);
        }

        for j in 1..=n {
            let curr = j % 2;
            let prev = 1 - curr;

            // Initialize the column
            for contig in &mut self.contigs {
                contig.aligner.init_column(j, curr, contig.len(), n);
            }

            // pre-compute the inter-contig jump scores for each contig
            let inter_contig_jump_infos = self
                .contigs
                .iter()
                .map(|c| {
                    let mut info = c.aligner.get_jump_info(
                        c.len(),
                        j - 1,
                        c.aligner.scoring.jump_score_inter_contig,
                    );
                    info.idx = c.aligner.contig_idx;
                    info
                })
                .collect_vec();

            // Get the best jump for each contig
            let mut best_jump_infos = Vec::new();
            for contig in &self.contigs {
                // get the contig index of the opposite strand of the current contig, if it exists
                let opp_contig_idx = self
                    .hashmap_for_strand(!contig.is_forward)
                    .get(&contig.name)
                    .copied();

                // Evaluate three jumps
                // 1. jump to the same contig and strand
                // 2. jump to the same contig and opposite strand
                // 3. jump to a different contig and any strand
                let same: JumpInfo = Self::jump_info_for_contig(contig, j);
                let flip_strand: Option<JumpInfo> =
                    self.jump_info_for_opposite_strand(opp_contig_idx, j);
                let inter_contig = Self::jump_info_for_inter_contig(
                    contig,
                    &inter_contig_jump_infos,
                    opp_contig_idx,
                );

                // NB: in case of ties, prefer a jump to the same contig and strand, then same
                // contig, then inter-contig
                let mut best_jump_info = same;
                if let Some(jump_info) = flip_strand {
                    if jump_info.score > best_jump_info.score {
                        best_jump_info = jump_info;
                    }
                }
                if let Some(jump_info) = inter_contig {
                    if jump_info.score > best_jump_info.score {
                        best_jump_info = jump_info;
                    }
                }
                best_jump_infos.push(best_jump_info);
            }

            // Fill in the column
            for contig in &mut self.contigs {
                contig.aligner.fill_column(
                    contig.seq,
                    y,
                    contig.len(),
                    n,
                    j,
                    prev,
                    curr,
                    best_jump_infos[contig.aligner.contig_idx as usize],
                );
            }
        }

        for contig in &mut self.contigs {
            contig
                .aligner
                .fill_last_column_and_end_clipping(contig.len(), n);
        }

        let aligners = self
            .contigs
            .iter()
            .map(|contig| &contig.aligner)
            .collect_vec();
        traceback(&aligners, n)
    }
}

// Tests
#[cfg(test)]
pub mod tests {
    use bio::alignment::pairwise::MatchParams;
    use itertools::Itertools;
    use rstest::rstest;

    use crate::{
        align::{aligners::constants::MIN_SCORE, scoring::Scoring},
        util::dna::reverse_complement,
    };

    use super::{Alignment, MultiContigAligner};

    /// Upper-cases and remove display-related characters from a string.
    fn s(bases: &str) -> Vec<u8> {
        bases
            .chars()
            .filter(|base| *base != '-' && *base != ' ' && *base != '_')
            .map(|base| base.to_ascii_uppercase() as u8)
            .collect_vec()
    }

    fn assert_alignment(
        alignment: &Alignment,
        xstart: usize,
        xend: usize,
        ystart: usize,
        yend: usize,
        score: i32,
        contig_idx: usize,
        cigar: &str,
        length: usize,
    ) {
        assert_eq!(alignment.xstart, xstart, "xstart {alignment}");
        assert_eq!(alignment.xend, xend, "xend {alignment}");
        assert_eq!(alignment.ystart, ystart, "ystart {alignment}");
        assert_eq!(alignment.yend, yend, "yend {alignment}");
        assert_eq!(alignment.score, score, "score {alignment}");
        assert_eq!(alignment.contig_idx, contig_idx, "contig_idx {alignment}");
        assert_eq!(alignment.cigar(), cigar, "cigar {alignment}");
        assert_eq!(alignment.length, length, "length {alignment}");
    }

    fn scoring_global_custom(
        mismatch_score: i32,
        gap_open: i32,
        gap_extend: i32,
        jump_score: i32,
    ) -> Scoring<MatchParams> {
        let match_fn = MatchParams::new(1, mismatch_score);
        let mut scoring = Scoring::with_jump_score(gap_open, gap_extend, jump_score, match_fn);
        scoring.xclip_prefix = MIN_SCORE;
        scoring.xclip_suffix = MIN_SCORE;
        scoring.yclip_prefix = MIN_SCORE;
        scoring.yclip_suffix = MIN_SCORE;
        scoring
    }

    fn scoring_global() -> Scoring<MatchParams> {
        scoring_global_custom(-1, -5, -1, -10)
    }

    fn scoring_local_custom(
        mismatch_score: i32,
        gap_open: i32,
        gap_extend: i32,
        jump_score: i32,
    ) -> Scoring<MatchParams> {
        let match_fn = MatchParams::new(1, mismatch_score);
        let mut scoring = Scoring::with_jump_score(gap_open, gap_extend, jump_score, match_fn);
        scoring.xclip_prefix = 0;
        scoring.xclip_suffix = 0;
        scoring.yclip_prefix = 0;
        scoring.yclip_suffix = 0;
        scoring
    }

    /// Identical sequences, all matches
    #[rstest]
    fn test_identical() {
        let x = s("ACGTAACC");
        let x_revcomp = reverse_complement(&x);
        let y = s("ACGTAACC");
        let mut aligner = MultiContigAligner::new();
        aligner.add_contig("fwd", true, &x, false, scoring_global());
        aligner.add_contig("revcomp", false, &x_revcomp, false, scoring_global());
        let alignment = aligner.custom(&y);
        assert_alignment(&alignment, 0, 8, 0, 8, 8, 0, "8=", 8);
    }

    /// Identical sequences, all matches, reverse complemented
    #[rstest]
    fn test_identical_revcomp() {
        let x = s("ACGTAACC");
        let x_revcomp = reverse_complement(&x);
        let y = reverse_complement(s("ACGTAACC"));
        let mut aligner = MultiContigAligner::new();
        aligner.add_contig("fwd", true, &x, false, scoring_global());
        aligner.add_contig("revcomp", false, &x_revcomp, false, scoring_global());
        let alignment = aligner.custom(&y);
        assert_alignment(&alignment, 0, 8, 0, 8, 8, 1, "8=", 8);
    }

    #[rstest]
    fn test_fwd_to_fwd_jump() {
        let x = s("AAGGCCTT");
        let x_revcomp = reverse_complement(&x);
        let y = s("AACCGGTT");
        let mut aligner = MultiContigAligner::new();
        aligner.add_contig(
            "fwd",
            true,
            &x,
            false,
            scoring_global_custom(-1, -100_000, -100_000, -1),
        );
        aligner.add_contig(
            "revcomp",
            false,
            &x_revcomp,
            false,
            scoring_global_custom(-1, -100_000, -100_000, -1),
        );
        let alignment = aligner.custom(&y);
        assert_alignment(
            &alignment,
            0,
            8,
            0,
            8,
            8 - 1 - 1 - 1,
            0,
            "2=2J2=4j2=2J2=",
            8,
        );
    }

    #[rstest]
    fn test_fwd_to_rev_jump() {
        let x = s("AACCTTGG");
        let x_revcomp = reverse_complement(&x); // CCAAGGTT
        let y = s("AACCGGTT");
        let mut aligner = MultiContigAligner::new();
        aligner.add_contig(
            "fwd",
            true,
            &x,
            false,
            scoring_global_custom(-100_000, -100_000, -100_000, -1),
        );
        aligner.add_contig(
            "revcomp",
            false,
            &x_revcomp,
            false,
            scoring_global_custom(-100_000, -100_000, -100_000, -1),
        );
        let alignment = aligner.custom(&y);
        assert_alignment(&alignment, 0, 8, 0, 8, 8 - 1, 0, "4=1C0J4=", 8);
    }

    #[rstest]
    fn test_rev_to_fwd_jump() {
        let x = s("CCAAGGTT");
        let x_revcomp = reverse_complement(&x);
        let y = s("AACCGGTT");
        let mut aligner = MultiContigAligner::new();
        aligner.add_contig(
            "fwd",
            true,
            &x,
            false,
            scoring_global_custom(-100_000, -100_000, -100_000, -1),
        );
        aligner.add_contig(
            "revcomp",
            false,
            &x_revcomp,
            false,
            scoring_global_custom(-100_000, -100_000, -100_000, -1),
        );
        let alignment = aligner.custom(&y);
        assert_alignment(&alignment, 0, 8, 0, 8, 8 - 1, 1, "4=1c0J4=", 8);
    }

    #[rstest]
    fn test_fwd_to_rev_long_jump() {
        // x fwd: AACCAAAATTGG
        //        ||||
        // y    : AACCNNNNGGTT
        //                ||||
        // x rev: CCAA____GGTT
        let x = s("AACCAAAATTGG");
        let x_revcomp = reverse_complement(&x); // CCAATTTTGGTT
        let y = s("AACCGGTT");
        let mut aligner = MultiContigAligner::new();
        aligner.add_contig(
            "fwd",
            true,
            &x,
            false,
            scoring_global_custom(-100_000, -100_000, -100_000, -1),
        );
        aligner.add_contig(
            "revcomp",
            false,
            &x_revcomp,
            false,
            scoring_global_custom(-100_000, -100_000, -100_000, -1),
        );
        let alignment = aligner.custom(&y);
        assert_alignment(&alignment, 0, 12, 0, 8, 8 - 1, 0, "4=1C4J4=", 8);
    }

    #[rstest]
    fn test_rev_to_fwd_long_jump() {
        let x = s("CCAANNNNGGTT");
        let x_revcomp = reverse_complement(&x);
        let y = s("AACCGGTT");
        let mut aligner = MultiContigAligner::new();
        aligner.add_contig(
            "fwd",
            true,
            &x,
            false,
            scoring_global_custom(-100_000, -100_000, -100_000, -1),
        );
        aligner.add_contig(
            "revcomp",
            false,
            &x_revcomp,
            false,
            scoring_global_custom(-100_000, -100_000, -100_000, -1),
        );
        let alignment = aligner.custom(&y);
        assert_alignment(&alignment, 0, 12, 0, 8, 8 - 1, 1, "4=1c4J4=", 8);
    }

    #[rstest]
    fn test_many_contigs() {
        let x1 = s("TATATCCCCCTATATATATATATATATA");
        let x2 = s("ATATATTATATATATATATATATGGGGG");
        let x3 = s("AAAAA");
        let x4 = s("TTTTTTTTTTTTTTTT");
        let y1 = s("AAAAACCCCCGGGGGAAAAATTTTTTTTTTTTTTTT");
        // contig idx:       222220000011111222223333333333333333
        // [5=] on x3 (bases 0-4), ends at offset 5
        // [2c0J] jumps to contig x1, no change in offset
        // [5=] on x1 (bases 5-9), ends at offset 10
        // [1C13J] jumps to contig x2, moves 13 bases forward (offset 23)
        // [5=] on x2 (bases 23-27), ends at offset 28
        // [1C28j] jumps to contig x3, moves 28 bases backwards (offset 0)
        // [5=] on x3 (bases 0-4), ends at offset 5
        // [1C5j] jumps to contig x4, moves 5 bases backwards (offset 0)
        // [16=] on x4 (bases 0-15), ends at offset 16
        let mut aligner = MultiContigAligner::new();
        let xs = vec![x1, x2, x3, x4];
        for (i, x) in xs.iter().enumerate() {
            aligner.add_contig(
                &format!("contig-{i}").to_string(),
                true,
                x,
                false,
                scoring_local_custom(-100_000, -100_000, -100_000, -1),
            );
        }
        let alignment = aligner.custom(&y1);
        assert_alignment(
            &alignment,
            0,
            16,
            0,
            36,
            36 - 1 - 1 - 1 - 1,
            2,
            "5=2c0J5=1C13J5=1C28j5=1C5j16=",
            36,
        );
    }

    #[rstest]
    fn test_jump_scores() {
        // y1 requires a jump to align fully, but where it jumps depends on the jump scores.
        let x1 = s("AAAAATTTTTAAAAA");
        let x2 = reverse_complement(&x1); // TTTTTAAAAATTTTT
        let x3 = s("AAAAA");
        let y1 = s("AAAAAAAAAA");
        let mut aligner = MultiContigAligner::new();
        aligner.add_contig(
            "chr1",
            true,
            &x1,
            false,
            scoring_local_custom(-1, -100_000, -100_000, -1),
        );
        aligner.add_contig(
            "chr1",
            false,
            &x2,
            false,
            scoring_local_custom(-1, -100_000, -100_000, -1),
        );
        aligner.add_contig(
            "chr2",
            true,
            &x3,
            false,
            scoring_local_custom(-1, -100_000, -100_000, -1),
        );

        // make these into test cases?

        // jump to the same contig and strand is prioritized
        for mut contig in &mut aligner.contigs {
            contig.aligner.scoring = contig.aligner.scoring.set_jump_scores(-1, -2, -2);
        }
        let alignment = aligner.custom(&y1);
        assert_alignment(&alignment, 0, 15, 0, 10, 10 - 1, 0, "5=5J5=", 10);

        // jump to the same contig and opposite strand is prioritized
        // starts in the middle of x2, then jumps back to the start of x1
        for mut contig in &mut aligner.contigs {
            contig.aligner.scoring = contig.aligner.scoring.set_jump_scores(-2, -1, -2);
        }
        let alignment = aligner.custom(&y1);
        assert_alignment(&alignment, 5, 15, 0, 10, 10 - 1, 1, "5A5=1c5j5=", 10);

        // jump to a different contig is prioritized
        // starts by aligning to x3 fully, then jumping to x1 and alinging to the last 5bp of x1
        for mut contig in &mut aligner.contigs {
            contig.aligner.scoring = contig.aligner.scoring.set_jump_scores(-2, -2, -1);
        }
        let alignment = aligner.custom(&y1);
        assert_alignment(&alignment, 0, 15, 0, 10, 10 - 1, 2, "5=2c5J5=", 10);

        // jump to the same contig and strand is prioritized when the scores are the same
        for mut contig in &mut aligner.contigs {
            contig.aligner.scoring = contig.aligner.scoring.set_jump_scores(-1, -1, -1);
        }
        let alignment = aligner.custom(&y1);
        assert_alignment(&alignment, 0, 15, 0, 10, 10 - 1, 0, "5=5J5=", 10);

        // jump to the same contig and opposite is prioritized when the scores are the same
        // starts in the middle of x2, then jumps back to the start of x1
        for mut contig in &mut aligner.contigs {
            contig.aligner.scoring = contig.aligner.scoring.set_jump_scores(-2, -1, -1);
        }
        let alignment = aligner.custom(&y1);
        assert_alignment(&alignment, 5, 15, 0, 10, 10 - 1, 1, "5A5=1c5j5=", 10);
    }
}