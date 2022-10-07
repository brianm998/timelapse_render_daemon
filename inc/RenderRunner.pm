package RenderRunner;

use strict;

use parent 'ProcessPipe';

use Term::ANSIColor qw(:constants);

# an object that can be used to render an image sequence into a video

sub new {
  my ($class,
      $log,
      $ffmpeg_cmd,
      $output_dirname,
      $output_video_filename,
      $group,
      $start_callback,
      $successful_update_callback,
      $finished_callback,
      $group_finished_callback,
     )
      = @_;

  my $self = $class->SUPER::new($log, $ffmpeg_cmd, $group,
				$start_callback,
				$successful_update_callback,
				$finished_callback,
				$group_finished_callback);
  $self->{output_dirname} = $output_dirname;
  $self->{output_video_filename} = $output_video_filename;
  $self->{frame_num} = 0;

  return bless $self, $class;
}

sub start($) {
  my ($self) = @_;

  $self->SUPER::start();

  $self->{log}->timeLog($self->{output_video_filename}, MAGENTA."starting ".RESET."render of".BLUE." $self->{output_video_filename}".RESET, 1);
}

sub read_line($) {
 my ($self) = @_;

# $self->{log}->timeLog($self->{output_video_filename}, "RENDER_FRAME for $self->{output_video_filename}", 10);

 my $ret = undef;
 if(eof($self->{shell_input})) {
   $self->finish();
 } else {
   $/ = ""; # don't use newline for <> XXX explore 'local' for this more
   my $line = readline($self->{shell_input});
   $/ = "\n";
   if (defined $line) {
     #   $self->{log}->timeLog($self->{output_video_filename}, "RENDER_FRAME for $self->{output_video_filename} INSIDE", 10);

     # frame= 1569 fps=0.9 q=-0.0 Lsize=  286475kB time=00:00:52.26 bitrate=44900.6kbits/s speed=0.0292x 
     if ($line =~ /^frame\s*=\s*(\d+)\s+fps\s*=\s*([\d.]+)/) {
       $self->{frame_num} = $1;
       $self->{fps} = $2;

       my $successful_update_callback = $self->{successful_update_callback};
       &$successful_update_callback($self) if defined $successful_update_callback;

     } else {
#       $self->{log}->timeLog($self->{output_video_filename}, "RENDER_FRAME for $self->{output_video_filename} MISS '$line'", 10);
     }
     $ret = 0;
   } else {
     $self->{log}->timeLog($self->{output_video_filename}, "RENDER_FRAME for $self->{output_video_filename} OUTSIDE", 10);
   }
 }
 return $ret;
}

sub finish($) {
  my ($self) = @_;

  # $self->{log}->timeLog($self->{output_video_filename}, "FINISH for $self->{output_video_filename}", 10);

  my $full_filename = $self->full_output_video_filename();

  if ($self->output_video_exists()) {
    my $video_size = sizeStringOf($full_filename);
    $self->{log}->timeLog($self->{output_video_filename}, GREEN."rendered".CYAN." $video_size".RESET." $self->{frame_num} frame".BLUE." $full_filename".RESET, -1);
    $self->{result} = 1;
  } else {
    # failed, why?
    $self->{log}->timeLog($self->{output_video_filename}, "$full_filename render FAILED", -1);
    $self->{result} = 'error';	# XXX expose errors somehow
  }

  $self->SUPER::finish();
}

sub output_video_exists() {
  my ($self) = @_;

  return -e $self->full_output_video_filename();
}

sub full_output_video_filename() {
  my ($self) = @_;

  return "$self->{output_dirname}/$self->{output_video_filename}";
}


# a human readable string giving the size of a file (or total of a shell globbed list)
# XXX copied from backup.pl :(
# XXX fix this
sub sizeStringOf {
  my ($item) = @_;
  open DU, "du -ch $item |";
  $/ = "\n";
  while (<DU>) { return $1 if $_ =~ /^\s*([\d\w.]+)\s+total/; }
  close DU;
}

1;
